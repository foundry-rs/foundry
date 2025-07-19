#!/usr/bin/env bash

set -eou pipefail

echo "Deleting node_modules and bun.lock..."

rm -rf node_modules
rm -rf bun.lock

bun remove @foundry-rs/forge

echo "Cleanup complete."

# echo "Installing @foundry-rs/forge..."

# REGISTRY_URL=${REGISTRY_URL:-https://registry.npmjs.org} \

# bun add @foundry-rs/forge --no-cache --force
