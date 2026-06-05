# Codex App Manager

Cross-platform manager for installing, updating, and uninstalling mirrored official Codex desktop app payloads.

This repository is the manager app, not the Codex app payload. Codex MSIX and DMG files should remain external assets served by `codex-app-mirror`.

## Shape

- Frontend: React, TypeScript, Vite
- Shell: Tauri v2
- Backend: Rust command layer with domain models, app services, ports, and platform adapters

## Development

```powershell
npm install
npm run dev
npm run tauri:dev
```

## Commands

```powershell
npm run check
npm run build
npm run tauri:build
```

## Boundaries

- `src/` is the UI and state orchestration layer.
- `src-tauri/src/domain/` defines payload, target, install state, and operation models.
- `src-tauri/src/app/` contains application services and planning logic.
- `src-tauri/src/ports/` declares replaceable boundaries for installers and payload repositories.
- `src-tauri/src/adapters/` contains host/platform-specific choices.
- `src-tauri/src/commands.rs` is the narrow Tauri bridge.

## Payload Policy

- Windows should prefer official MSIX/App Installer flows.
- Windows fixed-path unpacked install is a fallback.
- macOS should use official DMG replacement with verification and rollback.
- User data is preserved by default during uninstall.

