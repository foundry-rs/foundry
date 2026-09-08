#!/usr/bin/env bash
set -euo pipefail

# Check downloads against Socket's central policy even when build jobs restore caches.
# Call the installed shim explicitly so toolchain setup cannot shadow it in PATH.
: "${SFW_SHIM_DIR:?Secure runner must install Socket Firewall first}"
if [[ ! -x "$SFW_SHIM_DIR/cargo" ]]; then
  echo "::error::Socket Firewall Cargo shim is missing."
  exit 1
fi

cd "${1:-.}"
git ls-files --error-unmatch Cargo.lock >/dev/null
git diff --exit-code HEAD -- Cargo.lock

socket_cache_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/socket-dependencies.XXXXXX")
trap 'rm -rf -- "$socket_cache_dir"' EXIT

# Git's CLI honors the certificate configuration supplied by Socket for Git fetches.
# Keep TLS verification enabled; Cargo's libgit2 transport failed it in CI.
CARGO_HOME="$socket_cache_dir" CARGO_NET_GIT_FETCH_WITH_CLI=true \
  "$SFW_SHIM_DIR/cargo" fetch --locked

git diff --exit-code HEAD -- Cargo.lock
