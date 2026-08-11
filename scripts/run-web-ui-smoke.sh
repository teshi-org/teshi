#!/usr/bin/env bash
# Build and inspect the supported GPUI WASM web distribution.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> build GPUI WASM frontend"
"$ROOT/scripts/build-teshi-web.sh"

DIST="$ROOT/apps/teshi-web/dist"
test -s "$DIST/index.html"
test -s "$DIST/main.js"
test -s "$DIST/pkg/teshi_web.js"
test -s "$DIST/pkg/teshi_web_bg.wasm"
rg -q 'name="teshi-ui-runtime" content="gpui-wasm"' "$DIST/index.html"

if rg -q 'teshi-web-ui|react-dom|__vite' "$DIST"; then
  echo "unsupported React runtime marker found in GPUI WASM dist" >&2
  exit 1
fi

echo "==> GPUI WASM distribution smoke passed"
