#!/usr/bin/env bash
set -euo pipefail

BASE_REF="${BASE_REF:-origin/master}"
CANDIDATE_REF="${CANDIDATE_REF:-HEAD}"
BENCHMARKS="${BENCHMARKS:-forge_test}"
REPOS="${REPOS:-ithacaxyz/account:v0.5.7}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%d%H%M%S)}"
BENCH_ROOT="${BENCH_ROOT:-/tmp/foundry-pr-bench-${RUN_ID}}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "${REPO_ROOT}"

git fetch origin master

mkdir -p "${BENCH_ROOT}/results"
git worktree add --detach "${BENCH_ROOT}/master" "${BASE_REF}"
git worktree add --detach "${BENCH_ROOT}/candidate" "${CANDIDATE_REF}"

cargo build --locked --release --bin foundry-bench

for label in master candidate; do
  (
    foundry_dir="${BENCH_ROOT}/${label}/.foundry"
    FOUNDRY_BENCH_WORKSPACE_ROOT="${BENCH_ROOT}/${label}" \
      FOUNDRY_BENCH_LOCAL_BUILD_PROFILE=profiling \
      FOUNDRY_DIR="${foundry_dir}" \
      PATH="${foundry_dir}/bin:${PATH}" \
      "${REPO_ROOT}/target/release/foundry-bench" \
        --versions local \
        --repos "${REPOS}" \
        --benchmarks "${BENCHMARKS}" \
        --output-dir "${BENCH_ROOT}/results" \
        --output-file "${label}.md" \
        --json-output "${label}.json" \
        --verbose
  )
done

printf 'Benchmark results written to %s/results\n' "${BENCH_ROOT}"
