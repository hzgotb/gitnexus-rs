#!/usr/bin/env bash
set -euo pipefail

ENGINE="${CONTAINER_ENGINE:-docker}"
IMAGE="${GITNEXUS_TEST_IMAGE:-gitnexus-rs-test}"
PLATFORM="${GITNEXUS_TEST_PLATFORM:-}"
TARGET_DIR="${GITNEXUS_TEST_TARGET_DIR:-/tmp/gitnexus-target}"
FORCE_REBUILD="${GITNEXUS_TEST_REBUILD:-0}"

if ! command -v "$ENGINE" >/dev/null 2>&1; then
  echo "Container engine not found: $ENGINE" >&2
  exit 1
fi

if [[ "$FORCE_REBUILD" == "1" ]]; then
  "$ENGINE" build -f Dockerfile.test -t "$IMAGE" .
elif ! "$ENGINE" image inspect "$IMAGE" >/dev/null 2>&1; then
  "$ENGINE" build -f Dockerfile.test -t "$IMAGE" .
fi

run_args=(
  --rm
  -v "$PWD:/workspace"
  -w /workspace
  -e "CARGO_TARGET_DIR=$TARGET_DIR"
)

if [[ -n "$PLATFORM" ]]; then
  run_args+=(--platform "$PLATFORM")
fi

if [[ -t 0 && -t 1 ]]; then
  run_args+=(-it)
fi

if [[ $# -eq 0 ]]; then
  cargo_args=(test)
else
  cargo_args=("$@")
fi

exec "$ENGINE" run "${run_args[@]}" "$IMAGE" cargo "${cargo_args[@]}"
