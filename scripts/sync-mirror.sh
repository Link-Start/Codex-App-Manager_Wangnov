#!/usr/bin/env bash
# Compatibility entry point for the release workflow. The Node implementation
# owns policy, direct backend verification, conditional promotion, and rollback;
# keeping this wrapper preserves the existing local/CI command surface.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec node "$ROOT/scripts/mirror-release.mjs" "$@"
