#!/usr/bin/env bash
# One-shot macOS distribution finalize for a freshly `tauri build`-ed bundle.
# Chains the whole post-build pipeline so CI (and you, locally) call ONE script:
#
#   1. inject the macOS 26 adaptive (Liquid Glass) icon      (adaptive-icon.sh)
#   2. inside-out Developer ID code-sign + hardened runtime  (sign-macos-app.sh)
#   3. notarize + staple                                     (notarize-macos.sh)
#   4. repackage the updater tarball (+ re-sign) from the finalized app
#   5. repackage the dmg so its embedded .app is the finalized, stapled one
#
# Run on macOS AFTER:  npm run tauri build [-- --target <triple>]
#
# Env (all via CI secrets / local env — never committed):
#   APPLE_SIGNING_IDENTITY   "Developer ID Application: NAME (TEAMID)"
#   AC_API_KEY_ID / AC_API_ISSUER_ID / AC_API_KEY   App Store Connect API key
#   TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]            updater signing key
# If AC_API_* is unset, notarization is skipped (handy for local dev finalizes).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-}"   # optional rust target triple, e.g. aarch64-apple-darwin
BUNDLE="$ROOT/src-tauri/target/${TARGET:+$TARGET/}release/bundle"

APP="$(/usr/bin/find "$BUNDLE/macos" -maxdepth 1 -name '*.app' 2>/dev/null | head -1)"
[[ -d "$APP" ]] || { echo "no .app under $BUNDLE/macos — build first" >&2; exit 1; }
NAME="$(basename "$APP" .app)"
log() { printf '\033[32m[finalize]\033[0m %s\n' "$*"; }

# 1) adaptive icon  2) sign  3) notarize/staple
bash "$ROOT/scripts/macos-adaptive-icon.sh" "$APP"
bash "$ROOT/scripts/sign-macos-app.sh" "$APP" "${APPLE_SIGNING_IDENTITY:--}"
if [[ -n "${AC_API_KEY_ID:-}" ]]; then
  bash "$ROOT/scripts/notarize-macos.sh" "$APP"
else
  log "AC_API_* not set — skipping notarization (dev finalize)"
fi

# 4) repackage updater tarball + signature from the finalized app
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  TAR="$BUNDLE/macos/$NAME.app.tar.gz"
  tar -C "$BUNDLE/macos" -czf "$TAR" "$NAME.app"
  npx --yes @tauri-apps/cli signer sign "$TAR" >/dev/null
  log "updater tarball repacked + re-signed: ${TAR##*/}"
else
  log "TAURI_SIGNING_PRIVATE_KEY not set — leaving updater tarball untouched"
fi

# 5) repackage dmg so its embedded copy is the finalized, stapled app
DMG="$(/usr/bin/find "$BUNDLE/dmg" -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)"
if [[ -n "$DMG" ]]; then
  rm -f "$DMG"
  hdiutil create -volname "$NAME" -srcfolder "$APP" -ov -format UDZO "$DMG" >/dev/null
  log "dmg repacked: ${DMG##*/}"
fi

log "done — every macOS artifact now carries the signed, stapled, adaptive-icon app."
