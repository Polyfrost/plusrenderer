#!/bin/sh
set -e
cd "$(dirname "$0")/.."
cargo +1.97 build -p plusrenderer-ffi --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/plusrenderer_ffi.wasm packages/renderer/plusrenderer.wasm
ls -lh packages/renderer/plusrenderer.wasm | awk '{ print $9, $5 }'
