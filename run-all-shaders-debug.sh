#!/usr/bin/env bash
# Open every shader in shaders/ at once (in parallel) with TISH_SHADER_DEBUG=1, using the pumped example.
# Builds it first if needed. Override concurrency: JOBS=8 ./run-all-shaders-debug.sh

set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -d shaders ]]; then
  echo "error: shaders/ not found (run this from the tish-shadertoy repo root)" >&2
  exit 1
fi

[[ -x target/pumped ]] || npm run build:pumped

JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)}"

find shaders -maxdepth 1 -type f -name '*.glsl' -print0 |
  xargs -0 -P "$JOBS" -I{} bash -c 'TISH_SHADER_DEBUG=1 TISH_SHADER_COMPAT_TANH=1 target/pumped "$1"' _ {}
