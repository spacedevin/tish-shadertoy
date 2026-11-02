#!/usr/bin/env bash
# Run tish-shadertoy-run with TISH_SHADER_DEBUG=1 for every file in shaders/ (in parallel).
# Override concurrency: JOBS=8 ./run-all-shaders-debug.sh

set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -d shaders ]]; then
  echo "error: shaders/ not found (run this from the tish-shadertoy repo root)" >&2
  exit 1
fi

JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)}"

find shaders -maxdepth 1 -type f -name '*.glsl' -print0 |
  xargs -0 -P "$JOBS" -I{} bash -c 'TISH_SHADER_DEBUG=1 TISH_SHADER_COMPAT_TANH=1 cargo run --bin tish-shadertoy-run -- src/main.tish "$1"' _ {}
