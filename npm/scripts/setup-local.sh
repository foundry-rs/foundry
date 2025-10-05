#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NPM_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$NPM_DIR/.." && pwd)"

REGISTRY_URL="${NPM_REGISTRY_URL:-http://localhost:4873}"
export NPM_REGISTRY_URL="$REGISTRY_URL"

# Parse host:port for npm config lookups
REGISTRY_HOSTPORT=$(echo "$REGISTRY_URL" | sed -E 's#^https?://##; s#/$##')

echo "Using registry: $REGISTRY_URL"

ensure_verdaccio() {
  echo "Checking Verdaccio at $REGISTRY_URL..." >&2
  if curl -fsS "$REGISTRY_URL/-/ping" >/dev/null 2>&1; then
    echo "Verdaccio is up." >&2
    # Ensure large uploads allowed (best-effort)
    if docker ps --format '{{.Names}}' | grep -qx 'verdaccio'; then
      if ! docker exec verdaccio sh -c "grep -q '^max_body_size:' /verdaccio/conf/config.yaml" 2>/dev/null; then
        echo "Configuring Verdaccio max_body_size: 300mb" >&2
        docker exec verdaccio sh -c "printf '\nmax_body_size: 300mb\n' >> /verdaccio/conf/config.yaml" || true
        docker restart verdaccio >/dev/null || true
        for i in {1..30}; do
          if curl -fsS "$REGISTRY_URL/-/ping" >/dev/null 2>&1; then
            break
          fi
          sleep 1
        done
      fi
    fi
    return 0
  fi

  if ! command -v docker >/dev/null 2>&1; then
    echo "Verdaccio not reachable and Docker not available to start it." >&2
    exit 1
  fi

  echo "Starting Verdaccio via Docker..." >&2
  # Reuse existing container if present
  if docker ps -a --format '{{.Names}}' | grep -qx 'verdaccio'; then
    docker start verdaccio >/dev/null
  else
    docker run -d --name verdaccio -p 4873:4873 verdaccio/verdaccio >/dev/null
  fi

  echo "Waiting for Verdaccio to become ready..." >&2
  
  for i in {1..60}; do
    if curl -fsS "$REGISTRY_URL/-/ping" >/dev/null 2>&1; then
      echo "Verdaccio is ready." >&2
      # Ensure large uploads allowed
      if ! docker exec verdaccio sh -c "grep -q '^max_body_size:' /verdaccio/conf/config.yaml" 2>/dev/null; then
        echo "Configuring Verdaccio max_body_size: 300mb" >&2
        docker exec verdaccio sh -c "printf '\nmax_body_size: 300mb\n' >> /verdaccio/conf/config.yaml" || true
        docker restart verdaccio >/dev/null || true
        for j in {1..30}; do
          if curl -fsS "$REGISTRY_URL/-/ping" >/dev/null 2>&1; then
            break
          fi
          sleep 1
        done
      fi
      return 0
    fi
    sleep 1
  done
  echo "Timed out waiting for Verdaccio at $REGISTRY_URL" >&2
  exit 1
}

ensure_npm_login() {
  echo "Checking npm authentication..." >&2
  if npm whoami --registry "$REGISTRY_URL" >/dev/null 2>&1; then
    echo "Already logged in to $REGISTRY_URL" >&2
  else
    local user="${NPM_USER:-foundry-rs}"
    local pass="${NPM_PASSWORD:-foundry-rs}"
    local mail="${NPM_EMAIL:-foundry-rs@example.com}"
    echo "Logging in to $REGISTRY_URL as '$user'..." >&2
    if ! printf "%s\n%s\n%s\n" "$user" "$pass" "$mail" | npm adduser --registry "$REGISTRY_URL" --scope=@foundry-rs; then
      echo "npm adduser failed. You can set NPM_USER/NPM_PASSWORD/NPM_EMAIL and retry." >&2
      exit 1
    fi
  fi

  # Export tokens for scripts that require them
  local token
  token=$(npm config get "//$REGISTRY_HOSTPORT/:_authToken" 2>/dev/null || true)
  if [[ -z "$token" || "$token" == "undefined" ]]; then
    echo "Could not read npm auth token from config for //$REGISTRY_HOSTPORT/." >&2
    echo "Continuing; npm may still use session auth, but scripts expect NPM_TOKEN." >&2
    token="localtesttoken"
  fi
  export NPM_TOKEN="$token"
  export NODE_AUTH_TOKEN="$token"
}

derive_platform() {
  ARCH=$(uname -m | awk '{print tolower($0)}')
  case "$ARCH" in
    aarch64) ARCH="arm64" ;;
    x86_64)  ARCH="amd64" ;;
  esac
  PLATFORM=$(uname -s | awk '{print tolower($0)}')
  FORGE_PACKAGE_NAME="@foundry-rs/forge-${PLATFORM}-${ARCH}"
}

build_wrappers_and_binary() {
  echo "Building npm wrappers (bun)" >&2
  (cd "$NPM_DIR" && bun install && bun run build)

  echo "Building forge binary (cargo)" >&2
  (cd "$REPO_ROOT" && cargo build --release -p forge)
}

stage_binary_for_package() {
  echo "Staging binary into $FORGE_PACKAGE_NAME" >&2
  (cd "$NPM_DIR" && PLATFORM_NAME="$PLATFORM" ARCH="$ARCH" bun ./scripts/prepublish.ts)
}

unpublish_if_present() {
  echo "Unpublishing from $REGISTRY_URL (if present)" >&2
  npm unpublish @foundry-rs/forge --registry "$REGISTRY_URL" --force || true
  npm unpublish "$FORGE_PACKAGE_NAME" --registry "$REGISTRY_URL" --force || true
}

publish_packages() {
  # npm config set max_body_size 300mb --registry "$REGISTRY_URL"

  echo "Publishing to $REGISTRY_URL" >&2
  (cd "$NPM_DIR" && bun scripts/publish.ts "$FORGE_PACKAGE_NAME")
  (cd "$NPM_DIR" && bun scripts/publish.ts @foundry-rs/forge)
}

ensure_verdaccio
ensure_npm_login
derive_platform
build_wrappers_and_binary
stage_binary_for_package
unpublish_if_present
publish_packages

echo "Done. Test with: npm_config_registry=$REGISTRY_URL npx @foundry-rs/forge --version" >&2
