#!/usr/bin/env bash
# Build the GPUI WASM shell into apps/teshi-web/dist for Pages/artifact
# diagnostics. Production `teshi web` opens the hosted Pages UI instead.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT_DIR="$ROOT/apps/teshi-web/dist"
PKG_DIR="$OUT_DIR/pkg"
CACHE_BUST="$(date -u +%Y%m%d%H%M%S)"

echo "==> building teshi-web (nightly, wasm32-unknown-unknown)"
rustup target add wasm32-unknown-unknown --toolchain nightly >/dev/null
cargo +nightly build --release --target wasm32-unknown-unknown -p teshi-web

WASM="$TARGET_DIR/wasm32-unknown-unknown/release/teshi_web.wasm"
if [[ ! -f "$WASM" ]]; then
  echo "missing $WASM" >&2
  exit 1
fi

echo "==> wasm-bindgen → $PKG_DIR"
rm -rf "$OUT_DIR"
mkdir -p "$PKG_DIR"
wasm-bindgen "$WASM" --target web --out-dir "$PKG_DIR"

cp "$ROOT/apps/teshi-web/web/index.html" "$OUT_DIR/index.html"
cp "$ROOT/apps/teshi-web/web/main.js" "$OUT_DIR/main.js"

# Keep Pages/static-host caches from serving an older entrypoint or WASM.
sed -i "s#new URL('teshi_web_bg.wasm', import.meta.url)#new URL('teshi_web_bg.wasm?v=${CACHE_BUST}', import.meta.url)#" \
  "$PKG_DIR/teshi_web.js"
sed -i "s#./pkg/teshi_web.js#./pkg/teshi_web.js?v=${CACHE_BUST}#" "$OUT_DIR/main.js"
sed -i "s#./pkg/teshi_web_bg.wasm#./pkg/teshi_web_bg.wasm?v=${CACHE_BUST}#" "$OUT_DIR/main.js"
sed -i "s#src=\"./main.js\"#src=\"./main.js?v=${CACHE_BUST}\"#" "$OUT_DIR/index.html"

echo "==> done: $OUT_DIR (cache bust $CACHE_BUST)"
echo "Diagnostic artifact ready at: $OUT_DIR"
