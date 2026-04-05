#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v zig >/dev/null 2>&1; then
  ZIG_CMD=(zig)
elif command -v mise >/dev/null 2>&1; then
  ZIG_CMD=(mise x -- zig)
else
  echo "Error: zig is not installed and mise is unavailable." >&2
  exit 1
fi

cd "$ROOT_DIR/packages/cli"

exec "${ZIG_CMD[@]}" build \
  --build-file build.zig \
  --cache-dir .zig-cache \
  --global-cache-dir .zig-global-cache \
  --prefix zig-out \
  "$@"
