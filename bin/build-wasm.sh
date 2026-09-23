#!/usr/bin/env sh

set -eu
cargo build -p house-edge-engine-wasm \
	--target wasm32-unknown-unknown \
	--profile release-wasm

cp target/wasm32-unknown-unknown/release-wasm/house_edge_engine_wasm.wasm \
	priv/static/engine.wasm
