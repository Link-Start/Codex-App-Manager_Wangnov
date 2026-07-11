# Release & code-signing

## Release notes

每个版本的 release note 写在 `docs/releases/v<X.Y.Z>.md`,随版本号 bump 一起进发版 PR;
tag 推送后 `release.yml` 按 tag 名取用该文件作为 GitHub Release 正文,并自动追加
"What's Changed" 与 Full Changelog。文件缺失时回退到 `docs/releases/FALLBACK.md`
(安装表 + 升级说明),正文永远不会为空。写法与双语风格见 `docs/releases/TEMPLATE.md`。

Cross-platform release is tag-driven via [`.github/workflows/release.yml`](../.github/workflows/release.yml).
The publish job normally consumes the full four-platform build matrix. Windows
tag jobs currently fail closed while the SignPath Foundation application and
trusted-build migration are pending, so a new tag cannot publish a partial or
unsigned release. See the [code signing policy](./code-signing-policy.md).

## macOS signing pipeline

`tauri build` deliberately runs **without** a `signingIdentity`. The bundle is
finalized afterward, because the vendored Sparkle `BinaryDelta` helper must be
signed inside-out and notarization rejects an ad-hoc nested binary. The chain
(wrapped by [`scripts/finalize-macos.sh`](../scripts/finalize-macos.sh)):

1. **Adaptive icon** — [`macos-adaptive-icon.sh`](../scripts/macos-adaptive-icon.sh)
   compiles `assets/icon.icon` with `actool` into `Assets.car`, injects it, and
   sets `CFBundleIconName` so macOS 26 follows the system appearance
   (Default / Dark / Clear / Tinted). Older macOS falls back to the static `.icns`.
2. **Sign** — [`sign-macos-app.sh`](../scripts/sign-macos-app.sh) signs every
   nested Mach-O first, then the outer app, with hardened runtime + entitlements.
3. **Notarize + staple** — [`notarize-macos.sh`](../scripts/notarize-macos.sh)
   submits with `notarytool` (App Store Connect API key) and staples the ticket.
4. **Repackage** — the updater `.app.tar.gz` is rebuilt + re-signed and the dmg
   is rebuilt so every artifact carries the finalized, stapled app.

### Local finalize

```bash
npm run tauri build                 # or: npm run tauri:build:mac (build + icon)
export APPLE_SIGNING_IDENTITY="Developer ID Application: NAME (TEAMID)"
export AC_API_KEY_ID=... AC_API_ISSUER_ID=... AC_API_KEY=/path/AuthKey.p8
export TAURI_SIGNING_PRIVATE_KEY=...   # updater key (optional locally)
bash scripts/finalize-macos.sh aarch64-apple-darwin
```

Omit the `AC_API_*` vars to skip notarization for a quick local dev finalize.

## Manager updater trust and size limits

`latest.json` contains provider-specific download URLs, so the R2/IHEP copy is
intentionally not byte-identical to GitHub's copy and is not itself a trust
anchor. `gen-updater-manifest.mjs` also emits a deterministic, URL-free
`release-identity.json` containing the version, release-note hash, exact target
artifact name, Tauri minisign signature, and SHA-256. The release workflow signs
that identity with the existing Tauri updater key and publishes the identity and
`.sig` as immutable `<version>/` assets on GitHub, R2, and IHEP.

The client still checks the mainland-friendly mirror first. Before displaying or
installing a candidate, it bounded-fetches the same source's versioned identity
and signature, verifies them with the pinned updater public key, and binds every
security-relevant manifest field to that identity. Artifact bytes then pass both
the normal Tauri minisign check and the signed SHA-256 before `Update::install`.
Any missing, oversized, invalid, or mismatched mirror response falls through to
the GitHub endpoint; installation still occurs at most once.

Hard in-memory limits are 256 KiB for manifests/identity, 16 KiB for the identity
signature, and 64 MiB for updater artifacts. For comparison, v0.3.1's largest
updater artifact is about 10.9 MiB. The exact vendored
`tauri-plugin-updater` 2.10.1 patch and its upgrade checklist live in
[`vendor/tauri-plugin-updater-2.10.1/VENDORED.md`](../vendor/tauri-plugin-updater-2.10.1/VENDORED.md).

Because Tauri's minisign trusted comment contains a timestamp, a rerun would
normally create a byte-different identity `.sig`. The workflow first reuses and
verifies an existing GitHub Release asset (then the mirror copy), and the mirror
sync refuses to overwrite byte-different versioned identity objects.

## Windows

Windows has no light/dark adaptive app icon (`.ico` is static) — the single
Default icon is used. NSIS installer + updater bundle are produced by `tauri
build`; no Apple-style finalize step.

Windows Authenticode is in an explicit **application/migration pending** state:

1. The project is applying to SignPath Foundation; approval, certificate
   issuance, and trusted-build project identifiers do not exist yet.
2. `release.yml` runs `assert-signpath-foundation-ready.ps1` before a Windows
   tag build. The script intentionally always fails and has no variable/secret
   bypass. Because publish depends on the complete matrix, this blocks the
   whole release rather than shipping an unsigned or macOS-only set.
3. No `WINDOWS_CERTIFICATE` PFX, password, thumbprint config, or local
   `signCommand` is supported. SignPath Foundation signs submitted trusted-build
   artifacts and requires per-request human approval; it is not a PFX provider.
4. A future PR must prove the NSIS packaging/signing order with real SignPath
   pilot artifacts, exclude direct signing of third-party plugins, and verify
   the final setup, installed app, uninstaller, timestamp, updater `.sig`, and
   mirror hashes before replacing the blocker.
5. The existing `required` Authenticode and packaged-smoke steps are a
   post-integration verification contract, not evidence that signing is active.
   Removing the blocker alone still leaves unsigned output failing closed.

PR-time x64 packaged smoke lives in
[`win-installer-check.yml`](../.github/workflows/win-installer-check.yml)
(`scripts/windows-packaged-smoke.ps1`: install → launch → upgrade → uninstall).
Required CI (`ci.yml`) also runs standalone engine crate tests for
`codex-mac-engine` and `codex-win-engine`.

## Required GitHub Actions secrets

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | base64 of your `Developer ID Application` **.p12** |
| `APPLE_CERTIFICATE_PASSWORD` | password for that .p12 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: NAME (TEAMID)` |
| `KEYCHAIN_PASSWORD` | any throwaway password for the temp keychain |
| `AC_API_KEY_ID` | App Store Connect API key id |
| `AC_API_ISSUER_ID` | App Store Connect issuer id |
| `AC_API_KEY_BASE64` | base64 of the `AuthKey_XXXX.p8` |
| `TAURI_SIGNING_PRIVATE_KEY` | updater private key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password (empty if none) |

There are currently **no Windows signing secrets or variables to configure**.
After Foundation approval, use only the exact organization/project/signing
policy/artifact-configuration values provisioned by SignPath, in a separately
reviewed trusted-build integration. Do not guess names and do not add a PFX
fallback. Export Apple `.p12` / `.p8` files to base64 with
`base64 -i file -o -`.

> Artifact globs in the workflow's *Collect* step and the matcher regexes in
> `gen-updater-manifest.mjs` assume the default Tauri bundler output names —
> adjust them if your `productName`/bundler config changes the filenames.
>
> Keep **updater signature**, **Authenticode**, and **SmartScreen reputation**
> conceptually separate. Current Windows installers remain unsigned and new tag
> publication is blocked until the SignPath migration is proven. See
> [`docs/windows-signing.md`](./windows-signing.md), the
> [code signing policy](./code-signing-policy.md), and the
> [privacy policy](./privacy.md).
