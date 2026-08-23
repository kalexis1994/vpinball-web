#!/usr/bin/env bash
# Builds the player to wasm and generates the bindings inside the Vite project.
set -euo pipefail

PROFILE="${1:-release}"
case "$PROFILE" in
  release) CARGO_FLAGS="--release"; OUT_DIR="target/wasm32-unknown-unknown/release" ;;
  debug)   CARGO_FLAGS="";          OUT_DIR="target/wasm32-unknown-unknown/debug" ;;
  *) echo "usage: $0 [release|debug]" >&2; exit 1 ;;
esac

cargo build -p vpw-player --target wasm32-unknown-unknown $CARGO_FLAGS

wasm-bindgen \
  --target web \
  --out-dir web/src/wasm \
  "$OUT_DIR/vpw_player.wasm"

echo
echo "done -> web/src/wasm/  ($(du -h web/src/wasm/vpw_player_bg.wasm | cut -f1))"
echo "serve with:  npm --prefix web run dev"
