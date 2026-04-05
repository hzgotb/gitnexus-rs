#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLI_DIR="$ROOT_DIR/packages/cli"
DIST_DIR="$ROOT_DIR/dist"

if command -v zig >/dev/null 2>&1; then
  ZIG_CMD=(zig)
elif command -v mise >/dev/null 2>&1; then
  ZIG_CMD=(mise x -- zig)
else
  echo "Error: zig is not installed and mise is unavailable." >&2
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

TMP_PREFIX_DIR="$(mktemp -d "${TMPDIR:-/tmp}/gitnexus-release.XXXXXX")"
trap 'rm -rf "$TMP_PREFIX_DIR"' EXIT

for target in "${TARGETS[@]}"; do
  target_prefix="$TMP_PREFIX_DIR/$target"
  artifact="$target_prefix/bin/gitn"
  out="$DIST_DIR/gitn-$target"

  if [[ "$target" == *windows* ]]; then
    artifact="${artifact}.exe"
    out="${out}.exe"
  fi

  echo "Building gitn for ${target} -> ${out}"
  (
    cd "$CLI_DIR"
    "${ZIG_CMD[@]}" build \
      --build-file build.zig \
      --cache-dir .zig-cache \
      --global-cache-dir .zig-global-cache \
      --prefix "$target_prefix" \
      -Dtarget="$target" \
      -Doptimize=ReleaseSafe
  )

  cp "$artifact" "$out"
done

echo "Done. Binaries are in: $DIST_DIR"
