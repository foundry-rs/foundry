#!/usr/bin/env bash

set -eou pipefail

# build
npm run build

# unpublish
npm unpublish @foundry-rs/forge --registry http://localhost:4873 --force || true
npm unpublish @foundry-rs/forge-darwin-arm64 --registry http://localhost:4873 --force || true

# publish
bun scripts/publish.ts @foundry-rs/forge
bun scripts/publish.ts @foundry-rs/forge-darwin-arm64
