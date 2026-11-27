#!/bin/sh
set -e
cd "$(dirname "$0")/.."
cargo +1.97 build -p plusrenderer-ffi --profile wasm --target wasm32-unknown-unknown
built=target/wasm32-unknown-unknown/wasm/plusrenderer_ffi.wasm
out=packages/renderer/plusrenderer.wasm
wasm_opt="${WASM_OPT:-wasm-opt}"
if command -v "${wasm_opt%% *}" >/dev/null 2>&1; then
    $wasm_opt "$built" -o "$out" -Oz --converge \
        --enable-sign-ext --enable-mutable-globals --enable-nontrapping-float-to-int \
        --enable-bulk-memory --enable-multivalue --enable-reference-types
else
    echo "wasm-opt not found, copying the wasm unoptimised (set WASM_OPT or put binaryen on PATH)" >&2
    cp "$built" "$out"
fi
ls -lh "$out" | awk '{ print $9, $5 }'
