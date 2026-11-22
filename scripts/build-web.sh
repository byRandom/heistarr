#!/usr/bin/env bash
# Builds the web UI into crates/web/dist (served by the server).
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version 0.2.100
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${1:-release}"
FLAG=""; [ "$PROFILE" = "release" ] && FLAG="--release"
cargo build $FLAG -p heistarr-web --target wasm32-unknown-unknown
OUT=crates/web/dist
rm -rf "$OUT" && mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  "target/wasm32-unknown-unknown/$PROFILE/heistarr_web.wasm"
if command -v wasm-opt >/dev/null; then
  wasm-opt -Oz -o "$OUT/heistarr_web_bg.wasm" "$OUT/heistarr_web_bg.wasm"
fi
cp crates/web/static/* "$OUT/"
cp style/heistarr.css "$OUT/"
echo "web UI built in $OUT"
