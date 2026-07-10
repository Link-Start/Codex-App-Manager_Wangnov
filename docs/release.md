# Release & code-signing

## Release notes

每个版本的 release note 写在 `docs/releases/v<X.Y.Z>.md`,随版本号 bump 一起进发版 PR;
tag 推送后 `release.yml` 按 tag 名取用该文件作为 GitHub Release 正文,并自动追加
"What's Changed" 与 Full Changelog。文件缺失时回退到 `docs/releases/FALLBACK.md`
(安装表 + 升级说明),正文永远不会为空。写法与双语风格见 `docs/releases/TEMPLATE.md`。

Cross-platform release is tag-driven via [`.github/workflows/release.yml`](../.github/workflows/release.yml).
Push a `v*` tag and CI builds, signs, notarizes, and publishes a GitHub Release
with the Tauri updater manifest.

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

## Windows

Windows has no light/dark adaptive app icon (`.ico` is static) — the single
Default icon is used. NSIS installer + updater bundle are produced by `tauri
build`; no Apple-style finalize step.

Post-build on the Windows matrix (see [`release.yml`](../.github/workflows/release.yml)):

1. **PE arch diagnostic** — `scripts/windows-pe-arch.ps1` records x64 vs ARM64
   machine types. ARM64 is cross-built on x64 runners; this is **not** runtime
   verification ([`docs/windows-signing.md`](./windows-signing.md)).
2. **Optional Authenticode** — `scripts/sign-windows-authenticode.ps1` signs the
   final `-setup.exe` when `WINDOWS_CERTIFICATE` is present; otherwise skips.
3. **Authenticode verify** — `scripts/verify-windows-authenticode.ps1` in
   `optional` mode by default; set `AUTHENTICODE_REQUIRED=true` to gate.
4. **Tauri updater `.sig`** — `npx tauri signer sign` (always required for
   Windows in-app update entries in `latest.json`).
5. **Collect final artifacts** — space-stripped names under `dist-artifacts/`,
   with an explicit check that both `-setup.exe` and `-setup.exe.sig` exist.

PR-time x64 packaged smoke lives in
[`win-installer-check.yml`](../.github/workflows/win-installer-check.yml)
(`scripts/windows-packaged-smoke.ps1`: install → launch → upgrade → uninstall).
Required CI (`ci.yml`) also runs standalone engine crate tests for
`codex-mac-engine` and `codex-win-engine`.

## Mirror promotion safety

Stable releases use a stage/verify/promote, dual-backend protocol implemented by
[`scripts/mirror-release.mjs`](../scripts/mirror-release.mjs):

1. Before uploading anything immutable, cryptographically verify every local
   updater payload against its manifest signature and the updater public key in
   `tauri.conf.json`. The manifest signature must also equal the local `.sig`
   sidecar. This gate runs for prereleases as well as stable releases.
2. Upload immutable, versioned artifacts and a run-specific candidate manifest
   to both R2 and IHEP. A versioned object is never overwritten.
3. Before publishing the GitHub Release, download the candidate and **every staged
   artifact** directly from each S3 endpoint. Verify every file's byte size and
   SHA-256, plus each updater bundle's embedded Tauri/minisign signature with the
   public key in `tauri.conf.json`. This includes both DMG installers even though
   they are not referenced by the updater manifest. Stable promotion requires all
   four updater platforms and requires every manifest signature to match its
   downloaded `<artifact>.sig` sidecar; `ALLOW_PARTIAL_RELEASE` cannot weaken the
   mirror gate. The same verify-only phase forces separate downloads of the
   run-specific candidate and all four updater payloads through the public
   Worker's R2 and mainland-China IHEP branches. Each response must identify the
   requested backend, catching bad routes, bucket bindings, Worker secondary
   credentials, and presigned redirects before immutable publication.
4. Publish the draft GitHub Release and require GitHub to report that it is
   immutable with canonical asset digests.
5. Repeat the complete direct-backend and public-route readback immediately before
   promotion, then read each backend's current `latest.json` and compare semantic
   versions. Newer candidates advance, same-version reruns are
   no-write/idempotent, and older tags fail closed.
6. Publish with ETag (`If-Match` / `If-None-Match`) conditions. If the second
   backend or final verification fails, restore every backend already changed,
   also using ETag conditions so rollback cannot overwrite a concurrent writer.

The release workflow has one repository-wide `release-latest-*` concurrency lane
with `queue: max`, so every pending tag remains queued instead of a third tag
replacing the second. The object-store conditions remain the independent backstop
for manual/external writes. `mirror-stage-summary.json`,
`mirror-verification-summary.json`, and `mirror-promotion-summary.json` are shown
in the job summary and retained as workflow artifacts for 90 days. If a runner is
hard-killed after only one
`latest.json` write, a fresh run recognizes the exact candidate identity, leaves
that backend untouched, and conditionally advances only the lagging backend.

Before `prepare` permits any build, and again at the start of every release-job
attempt, the workflow queries the repository Immutable Releases setting with a
dedicated fine-grained, read-only token. A missing token, failed query, or
`enabled: false` response fails closed before draft upload or mirror publication.

Same-tag reruns reuse the artifacts and `latest.json` attached to a complete,
published GitHub Release only when GitHub reports `immutable: true` and a canonical
`sha256:` digest for every required asset. The workflow re-hashes every downloaded
asset against those API digests, and rejects a mutable release instead of calling
its current bytes canonical. Only a draft is repairable; a published mutable or
incomplete Release fails closed before any asset upload. The release job performs
this lookup live rather than
trusting `prepare` outputs from an earlier attempt, so **Re-run failed jobs** after
a mirror failure reuses the already-published immutable bytes. While a new or
partial release is still being repaired, the workflow selects the earliest
successful, attempt-qualified Actions artifact for each platform so every rerun
stages the same signed/notarized bytes. It never creates a second byte sequence for
an immutable version key. New releases are uploaded as drafts first and published
only after all assets succeed, including prereleases. Immediately after publish,
the workflow requires the Release itself to report `immutable: true` and canonical
digests for every required asset before any mirror pointer can advance. A
historical Actions run still executes its historical workflow revision, so the
release environment intentionally uses new `*_PROMOTION_*` credential names. The
four legacy access-key secrets listed below must be deleted; the current workflow
has a hard gate that rejects them if they are reintroduced.

### Emergency mirror downgrade

A normal old-tag rerun can never downgrade the mirror. For an incident rollback:

1. Open the current `Release` workflow and dispatch it from the repository's
   **default branch** (never select the historical tag's workflow revision).
2. Set `target_tag` to the already-published `vX.Y.Z` release to restore.
3. Enable `allow_mirror_downgrade`.
4. Enter a concrete `mirror_downgrade_reason` (at least 10 characters).

The workflow downloads the target release's original assets instead of rebuilding
them. The target GitHub Release must be immutable and expose canonical SHA-256
digests; older mutable releases must first be migrated through an explicitly
reviewed process and cannot be used directly. The script rejects the override on
tag-push events or without GitHub actor/run metadata. The triggering actor (plus
the original workflow actor on a rerun), reason, whether the override was actually
used, and the run URL are written to the promotion audit and job summary.

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
| `IMMUTABLE_RELEASES_READ_TOKEN` | fine-grained token used only to read this repository's Immutable Releases setting; grant **Administration: read-only** and store it in the `release` environment |
| `MANAGER_R2_S3_ENDPOINT` | R2 S3-compatible endpoint |
| `MANAGER_R2_PROMOTION_ACCESS_KEY_ID` | R2 write/read credential used only by the protected workflow |
| `MANAGER_R2_PROMOTION_SECRET_ACCESS_KEY` | R2 write/read secret used only by the protected workflow |
| `MANAGER_IHEP_S3_ENDPOINT` | IHEP S3-compatible endpoint |
| `MANAGER_IHEP_S3_BUCKET` | IHEP bucket |
| `MANAGER_IHEP_S3_REGION` | IHEP region; defaults to `auto` when empty |
| `MANAGER_IHEP_S3_PREFIX` | optional object-key prefix for IHEP |
| `MANAGER_IHEP_S3_PROMOTION_ACCESS_KEY_ID` | IHEP write/read credential used only by the protected workflow |
| `MANAGER_IHEP_S3_PROMOTION_SECRET_ACCESS_KEY` | IHEP write/read secret used only by the protected workflow |

After creating the promotion credentials, delete (and preferably revoke/rotate)
`MANAGER_R2_ACCESS_KEY_ID`, `MANAGER_R2_SECRET_ACCESS_KEY`,
`MANAGER_IHEP_S3_ACCESS_KEY_ID`, and `MANAGER_IHEP_S3_SECRET_ACCESS_KEY` from the
repository and `release` environment. Historical workflow revisions reference
those exact names; leaving any of them available defeats old-run isolation.

### Optional signing secrets and release variables

| Name | What |
|---|---|
| `WINDOWS_CERTIFICATE` | base64 of OV/EV code-signing **.pfx** (release env) |
| `WINDOWS_CERTIFICATE_PASSWORD` | password for that .pfx |
| `AUTHENTICODE_REQUIRED` (repo **variable**) | `true` → fail release when PE is not `Valid` |
| `WINDOWS_TIMESTAMP_URL` (repo **variable**) | optional RFC3161 timestamp URL |
| `MANAGER_R2_BUCKET` (repo **variable**) | R2 bucket; defaults to `codex-app-manager` |

Export your local .p12 / .p8 / .pfx to base64 with `base64 -i file -o -`.

> Artifact globs in the workflow's *Collect* step and the matcher regexes in
> `gen-updater-manifest.mjs` assume the default Tauri bundler output names —
> adjust them if your `productName`/bundler config changes the filenames.
>
> Keep **updater signature**, **Authenticode**, and **SmartScreen reputation**
> conceptually separate — see [`docs/windows-signing.md`](./windows-signing.md).
>
> Before enabling promotion, seed both S3-compatible endpoints with a valid
> `latest.json` baseline. Both endpoints must support conditional `PutObject`
> requests (`If-Match` / `If-None-Match`) and must preserve custom user metadata
> through `HeadObject`. Promotion fails closed if either baseline is absent or an
> endpoint cannot enforce those requirements.
