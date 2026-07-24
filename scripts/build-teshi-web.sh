#!/usr/bin/env bash
# Build GPUI WASM shell into apps/teshi-web/dist for Path 1 (daemon --dist).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT_DIR="$ROOT/apps/teshi-web/dist"
PKG_DIR="$OUT_DIR/pkg"

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

echo "==> done: $OUT_DIR"
echo "Path 1: cargo run -p teshi-cli -- web --no-open --dist \"$OUT_DIR\""
