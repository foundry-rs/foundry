#!/usr/bin/env bash

set -eou pipefail

echo "Deleting node_modules and bun.lock..."

rm -rf node_modules
rm -rf bun.lock

bun remove @foundry-rs/forge

echo "Cleanup complete."
