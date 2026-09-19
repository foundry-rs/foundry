#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 4 ]]; then
  echo "usage: $0 <project-root> <rng|jev> [timeout-seconds] [seed]" >&2
  exit 2
fi

project_root=$1
mode=$2
timeout_seconds=${3:-30}
seed=${4:-0x585f37fbac9620027325a193e979e3c93c87b069b15dce6c903f5f460436f916}
forge_bin=${FORGE_BIN:-forge}

if [[ $mode != rng && $mode != jev ]]; then
  echo "transaction generator must be 'rng' or 'jev'" >&2
  exit 2
fi
if [[ ! -d $project_root ]]; then
  echo "project root does not exist: $project_root" >&2
  exit 2
fi

run_dir=$(mktemp -d "${TMPDIR:-/tmp}/foundry-jev-scfuzz.XXXXXX")
corpus_dir="$run_dir/corpus"
failure_dir="$run_dir/failures"
log_file="$run_dir/forge.log"
mkdir -p "$corpus_dir" "$failure_dir"

echo "SCFUZZ_START mode=$mode seed=$seed timeout=$timeout_seconds run_dir=$run_dir"
started_at=$(date +%s)
set +e
(
  cd "$project_root"
  env \
    FOUNDRY_INVARIANT_TIMEOUT="$timeout_seconds" \
    FOUNDRY_INVARIANT_RUNS=500000000 \
    FOUNDRY_INVARIANT_DEPTH=100 \
    FOUNDRY_INVARIANT_CORPUS_DIR="$corpus_dir" \
    FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR="$failure_dir" \
    RUST_LOG=forge::jev=debug \
    "$forge_bin" test --mc CryticToFoundry --match-test 'invariant_' \
      --invariant-tx-generator "$mode" --invariant-workers 1 \
      --fuzz-seed "$seed" -v
) 2>&1 | tee "$log_file"
status=${PIPESTATUS[0]}
set -e
elapsed=$(($(date +%s) - started_at))
echo "SCFUZZ_END mode=$mode status=$status elapsed=$elapsed log=$log_file"
