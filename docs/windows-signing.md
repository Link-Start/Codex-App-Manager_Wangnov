# Windows signing and verification

The release workflow uses three independent trust mechanisms. Authenticode
identifies the Windows publisher, the Tauri updater signature protects the
bytes consumed by in-app updates, and SmartScreen reputation is a Microsoft
risk signal that neither signature can guarantee. Keep them separate when
triaging a warning or a failed release.

Historical releases keep the signing status recorded in their own release
notes. The mandatory pipeline below applies starting with the first release
built from this workflow revision.

## 中文

### 发布状态

- Windows 正式发布必须提供 OV/EV Authenticode `.pfx`；缺少证书或密码会直接阻断 tag 发布。
- x64 与 ARM64 的主程序、NSIS `uninstall.exe`、外层 `-setup.exe` 均由同一张证书签名，并使用 SHA-256 + RFC3161 时间戳。
- CI 要求每层 `Get-AuthenticodeSignature` 都为 `Valid`、签名证书 thumbprint 与本次导入证书完全一致，并存在时间戳 countersigner。
- PR 构建不会取得发布证书，仍以 unsigned 方式验证打包模板和安装生命周期；PR 的 optional 探测不代表正式发布可以未签名。
- Windows 应用内更新仍另外要求 Tauri updater `.sig`；它覆盖已经完成 Authenticode 签名的最终安装器字节。

### 三个独立概念

**Authenticode 代码签名**

Windows 对 PE 文件和安装器使用的发行者签名。本项目在 `tauri build` 前导入 PFX，并通过临时 Tauri config 配置 `certificateThumbprint`、`digestAlgorithm=sha256`、RFC3161 `timestampUrl` 与 `tsp=true`。Tauri 按 inside-out 顺序签主程序、NSIS 生成的卸载器和外层安装器。

**Tauri updater 签名**

`latest.json` 的 `signature` 对应用内更新所下载的最终安装器字节签名。它防止 GitHub、镜像或传输链路改包，但不向 Windows 声明发行者身份。实现使用 `TAURI_SIGNING_PRIVATE_KEY` 和 `npx tauri signer sign`；其私钥与 Authenticode PFX 完全不同。

**SmartScreen 信誉**

Microsoft Defender SmartScreen 会综合发行者、文件流行度、历史与其他风险信号。一个 Authenticode 签名可以让系统显示可验证的发行者，但不能承诺新证书或新工件绝不出现 SmartScreen 提示。看到 SmartScreen 提示也不等于 Tauri updater 签名失败。

### 发布硬门

Windows matrix 的顺序如下：

1. `prepare-windows-authenticode.ps1` 解码并导入 PFX，生成仅用于当前 job 的 Tauri signing config。
2. `tauri build --config ...` 完成三层 inside-out Authenticode 签名，并让自定义 NSIS 模板优先使用 bundler 准备的已签名插件目录。
3. `verify-windows-authenticode.ps1 -Mode required` 验证外层安装器。Tauri 打包结束后会把 `target/.../release/codex-app-manager.exe` 恢复成 unsigned、未 patch 的构建输入，因此这个 raw 中间文件只用于 PE 架构诊断，不作为签名硬门。
4. `windows-packaged-smoke.ps1` 安装包并直接验证安装后的主程序与 `uninstall.exe`。
   - x64 执行 `install → launch → upgrade → uninstall`。
   - ARM64 在 x64 runner 上执行安装、签名验证、升级和卸载，但跳过 ARM64 主程序启动；完整 ARM64 运行验收仍需 ARM64 设备或可信虚拟化。
5. 对 Authenticode 签名后的最终 `-setup.exe` 生成独立的 Tauri updater `.sig`。
6. 收集无空格的发布文件名后，再次对最终待发布安装器执行 required 验证。
7. 无论成功失败都移除临时证书和 PFX 目录。

任何一层缺失、`Status != Valid`、证书 thumbprint 不符、没有时间戳、没有 updater `.sig`，发布 job 都失败。RFC3161 同时由生成配置中的 `tsp=true` / SHA-256 / timestamp URL 和工件上的 timestamp countersigner 证明。

### 必需配置

在 GitHub `release` environment 配置：

| 名称 | 类型 | 用途 |
|---|---|---|
| `WINDOWS_CERTIFICATE` | secret | base64 编码的 OV/EV 代码签名 `.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | secret | PFX 密码；证书无密码时可以为空 |
| `WINDOWS_TIMESTAMP_URL` | variable，可选 | RFC3161 服务地址；默认 `http://timestamp.digicert.com` |
| `TAURI_SIGNING_PRIVATE_KEY` | secret | Tauri updater 私钥，不是 Authenticode 证书 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | updater 私钥密码 |

不再使用 `AUTHENTICODE_REQUIRED` 开关：tag release 永远是 required。不要把 PFX 暴露给 `pull_request` workflow，也不要把真实证书提交进仓库。

### 用户核验

在 Windows 上可直接查看三层签名：

```powershell
Get-AuthenticodeSignature .\CodexAppManager_x64-setup.exe | Format-List Status,SignerCertificate,TimeStamperCertificate
Get-AuthenticodeSignature "$env:LOCALAPPDATA\Codex App Manager\codex-app-manager.exe" | Format-List Status,SignerCertificate,TimeStamperCertificate
Get-AuthenticodeSignature "$env:LOCALAPPDATA\Codex App Manager\uninstall.exe" | Format-List Status,SignerCertificate,TimeStamperCertificate
```

发布页仍提供 `SHA256SUMS`，用于确认下载文件与 release 资产逐字节一致：

```powershell
Get-FileHash .\CodexAppManager_x64-setup.exe -Algorithm SHA256
Get-FileHash .\CodexAppManager_arm64-setup.exe -Algorithm SHA256
```

哈希、Authenticode、updater `.sig` 各自回答不同问题，不能互相替代。

### 相关脚本

- [`scripts/prepare-windows-authenticode.ps1`](../scripts/prepare-windows-authenticode.ps1) — 正式发布导入证书并生成 Tauri 配置；凭据缺失即失败。
- [`scripts/verify-windows-authenticode.ps1`](../scripts/verify-windows-authenticode.ps1) — `required` 发布硬门；`optional` 仅供无 secret 的 PR / 本地诊断。
- [`scripts/windows-packaged-smoke.ps1`](../scripts/windows-packaged-smoke.ps1) — 安装后验证主程序与卸载器，并覆盖升级、卸载路径。
- [`scripts/sign-windows-authenticode.ps1`](../scripts/sign-windows-authenticode.ps1) — 旧的本地单文件签名工具；正式发布不依赖它。
- [`scripts/windows-pe-arch.ps1`](../scripts/windows-pe-arch.ps1) — 验证 x64 / ARM64 PE machine type。

## English

### Release state

- Windows releases require an OV/EV Authenticode PFX. Missing credentials hard-fail the tag build.
- The x64 and ARM64 main executable, generated NSIS uninstaller, and outer setup executable are signed by the same certificate with SHA-256 and an RFC3161 timestamp.
- CI requires `Get-AuthenticodeSignature.Status == Valid`, an exact match to the imported certificate thumbprint, and a timestamp countersigner on every layer.
- Pull requests do not receive release secrets. They intentionally build unsigned packages and run optional probes plus lifecycle smoke; this does not make unsigned release artifacts acceptable.
- In-app updates separately require a Tauri updater `.sig` over the final Authenticode-signed setup bytes.

### Three independent mechanisms

**Authenticode** is Windows publisher identity for PE files. The release job imports the PFX and gives Tauri a private config with `certificateThumbprint`, SHA-256, an RFC3161 timestamp URL, and `tsp=true`. Tauri signs the main executable, generated uninstaller, and outer NSIS installer inside-out; the custom template also selects Tauri's private signed NSIS plugin directory.

**The Tauri updater signature** authenticates the final bytes referenced by `latest.json`. It protects downloads from tampering across GitHub, the mirror, and transport hops. It does not establish a Windows publisher. Its private key is separate from the Authenticode PFX.

**SmartScreen reputation** is a Microsoft risk signal based on more than the presence of a signature. Authenticode provides a verifiable publisher but cannot promise that a new certificate or artifact will never trigger a warning. A SmartScreen warning is also not evidence that the updater signature failed.

### Release gate

The Windows jobs prepare the certificate, build with inside-out signing, verify the outer setup executable, install each architecture's package to inspect the signed embedded main/uninstaller, create the independent updater signature, collect the final names, and verify the publishable setup again. Tauri restores `target/.../release/codex-app-manager.exe` to its unsigned, unpatched build input after bundling, so that raw intermediate is used only for the PE architecture diagnostic and is not a signature gate. x64 runs the full launch lifecycle; ARM64 performs install/signature/upgrade/uninstall checks on the x64 host but requires ARM64 hardware or trusted virtualization for a real launch test.

There is no `AUTHENTICODE_REQUIRED` toggle for a tag release. Any missing layer, non-`Valid` status, wrong thumbprint, absent timestamp, or missing updater signature blocks publication. The PFX and imported certificate are removed in an `always()` cleanup step.

### Required release environment

| Name | Kind | Purpose |
|---|---|---|
| `WINDOWS_CERTIFICATE` | secret | base64-encoded OV/EV code-signing PFX |
| `WINDOWS_CERTIFICATE_PASSWORD` | secret | PFX password; may be empty for an unprotected PFX |
| `WINDOWS_TIMESTAMP_URL` | optional variable | RFC3161 endpoint; defaults to `http://timestamp.digicert.com` |
| `TAURI_SIGNING_PRIVATE_KEY` | secret | Tauri updater private key, not the Authenticode certificate |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | updater-key password |

Never expose the PFX to a pull-request workflow or commit certificate material to the repository. Historical release notes retain the actual status of those older artifacts.
