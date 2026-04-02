#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIST_DIR="$ROOT_DIR/dist"

BIN_NAMES=(
  "gitn"
)

BIN_SRCS=(
  "$ROOT_DIR/packages/cli/src/main.zig"
)

if command -v zig >/dev/null 2>&1; then
  ZIG_CMD=(zig)
elif command -v mise >/dev/null 2>&1; then
  ZIG_CMD=(mise x -- zig)
else
  echo "Error: zig is not installed and mise is unavailable."
  exit 1
fi

mkdir -p "$DIST_DIR"

TARGETS=(
  "x86_64-macos"
  "aarch64-macos"
  "x86_64-linux-musl"
  "aarch64-linux-musl"
  "x86_64-windows-gnu"
  "aarch64-windows-gnu"
)

for i in "${!BIN_NAMES[@]}"; do
  name="${BIN_NAMES[$i]}"
  src="${BIN_SRCS[$i]}"

  for target in "${TARGETS[@]}"; do
    out="$DIST_DIR/${name}-${target}"
    if [[ "$target" == *windows* ]]; then
      out="${out}.exe"
    fi

    echo "Building ${name} for ${target} -> ${out}"
    "${ZIG_CMD[@]}" build-exe "$src" \
      -O ReleaseSafe \
      -target "$target" \
      -femit-bin="$out"
  done
done

echo "Done. Binaries are in: $DIST_DIR"
