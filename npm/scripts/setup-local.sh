#!/usr/bin/env bash

set -euo pipefail

REGISTRY_URL="${NPM_REGISTRY_URL:-http://localhost:4873}"
export NPM_REGISTRY_URL="$REGISTRY_URL"
export NPM_TOKEN="${NPM_TOKEN:-localtesttoken}"

bun run build

# create user if not exists
me=$(npm whoami --registry "$REGISTRY_URL")
if [ -z "$me" ]; then
  echo "Not logged in or no user. Please run 'npm adduser --registry $REGISTRY_URL --scope=@foundry-rs' to create a user."
  exit 1
fi

# determine arch and platform
ARCH=$(uname -m | awk '{print tolower($0)}')
if [ "$ARCH" = "aarch64" ]; then
    ARCH="arm64"
fi
if [ "$ARCH" = "x86_64" ]; then
    ARCH="amd64"
fi
PLATFORM=$(uname -s | awk '{print tolower($0)}')
FORGE_PACKAGE_NAME="@foundry-rs/forge-${PLATFORM}-${ARCH}"
echo "FORGE_PACKAGE_NAME: $FORGE_PACKAGE_NAME"


echo "Unpublishing from $REGISTRY_URL (if present)" >&2
npm unpublish @foundry-rs/forge --registry "$REGISTRY_URL" --force || true
npm unpublish "$FORGE_PACKAGE_NAME" --registry "$REGISTRY_URL" --force || true

echo "Publishing to $REGISTRY_URL" >&2
# Publish platform packages first
bun scripts/publish.ts "$FORGE_PACKAGE_NAME"
# Publish meta package last so optionalDependencies point to the same version
bun scripts/publish.ts @foundry-rs/forge
