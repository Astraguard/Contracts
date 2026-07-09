#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "Building all contracts for wasm32v1-none..."
cargo build --target wasm32v1-none --release

WASM_DIR="target/wasm32v1-none/release"

for wasm in "$WASM_DIR"/*.wasm; do
  name="$(basename "$wasm")"
  echo "Optimizing ${name}..."
  stellar contract optimize --wasm "$wasm"
done

echo "Done. Optimized wasm files are in ${WASM_DIR}."
