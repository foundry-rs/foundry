#!/usr/bin/env bash

set -eou pipefail

tools=(cast anvil forge chisel)

echo "Building tools…"

for tool in "${tools[@]}"; do
  cargo build \
    --package "$tool" \
    --target aarch64-apple-darwin
done

echo "Generating package.json files and moving binaries…"
for tool in "${tools[@]}"; do
  PLATFORM_NAME="darwin" ARCH="arm64" bun ./scripts/prepublish.mjs \
    --tool "$tool" --bin-path "../target/aarch64-apple-darwin/debug/$tool"
done