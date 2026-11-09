#!/usr/bin/env sh
# Build the engine for the browser and put it in the
# right spot for Phoenix

set -eu
cd "$(dirname "$0")/.."
cargo build -p house-edge-engine-wasm \
	--target wasm32-unknown-unknown \
	--profile release-wasm

cp target/wasm32-unknown-unknown/release-wasm/house_edge_engine_wasm.wasm \
	site/priv/static/engine.wasm
