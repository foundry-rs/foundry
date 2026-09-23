#!/usr/bin/env bash
# Detached workload: all descriptors point to files, never to a workflow step.
set -euo pipefail
diagnostic_dir="${RUNNER_TEMP:?}/macos-diagnostic"

mark() {
  printf '%s %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*" >> "$diagnostic_dir/lifecycle.log"
}

finish() {
  status=$?
  trap - EXIT
  mark "workload-exit status=$status"
  printf '%s\n' "$status" > "$diagnostic_dir/build-exit.tmp"
  mv "$diagnostic_dir/build-exit.tmp" "$diagnostic_dir/build-exit.txt"
  exit "$status"
}
trap finish EXIT

mark "workload-start mode=${DIAGNOSTIC_MODE:?}"
case "$DIAGNOSTIC_MODE" in
  smoke)
    # Exercise the same process handoff and cleanup without building or testing.
    sleep 15
    mark smoke-complete
    ;;
  build)
    cd "${GITHUB_WORKSPACE:?}/source"
    mark fetch-start
    cargo fetch --locked --target aarch64-apple-darwin
    mark fetch-end
    mark build-start
    /usr/bin/time -l cargo build --locked --target aarch64-apple-darwin \
      --profile dist --no-default-features \
      --features aws-kms,gcp-kms,turnkey,cli,asm-keccak,js-tracer,base,monad,optimism,touch-id \
      --features jemalloc --bins
    mark build-end
    ;;
  *) printf 'Unknown diagnostic mode\n' >&2; exit 2 ;;
esac
