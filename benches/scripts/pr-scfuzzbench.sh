#!/usr/bin/env bash
set -euo pipefail

BASE_REF="${BASE_REF:-origin/master}"
CANDIDATE_REF="${CANDIDATE_REF:-HEAD}"
TARGET_REPO="${TARGET_REPO:-https://github.com/Recon-Fuzz/aave-v4-scfuzzbench.git}"
TARGET_REF="${TARGET_REF:-v0.5.6-recon}"
BENCHMARK_TYPE="${BENCHMARK_TYPE:-property}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-3600}"
WORKERS="${WORKERS:-1}"
FOUNDRY_TEST_ARGS="${FOUNDRY_TEST_ARGS:-}"
PROPERTIES_PATH="${PROPERTIES_PATH:-}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%d%H%M%S)}"
BENCH_ROOT="${BENCH_ROOT:-/tmp/foundry-scfuzzbench-${RUN_ID}}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "${REPO_ROOT}"

if [[ -n "$(git status --porcelain)" ]]; then
  printf 'error: working directory is dirty; commit or stash changes before benchmarking\n' >&2
  exit 1
fi

git fetch origin master

mkdir -p "${BENCH_ROOT}"
git worktree add --detach "${BENCH_ROOT}/master" "${BASE_REF}"
git worktree add --detach "${BENCH_ROOT}/candidate" "${CANDIDATE_REF}"

(
  cd "${REPO_ROOT}"
  cargo build --locked --release --bin foundry-scfuzzbench
)

for label in master candidate; do
  (
    cd "${BENCH_ROOT}/${label}"
    cargo build --locked --profile profiling --bin forge
  )

  args=(
    --target-repo "${TARGET_REPO}"
    --target-ref "${TARGET_REF}"
    --benchmark-type "${BENCHMARK_TYPE}"
    --timeout-seconds "${TIMEOUT_SECONDS}"
    --workers "${WORKERS}"
    --output-dir "${BENCH_ROOT}/${label}-artifacts"
    --foundry-bin "${BENCH_ROOT}/${label}/target/profiling/forge"
  )

  if [[ -n "${FOUNDRY_TEST_ARGS}" ]]; then
    args+=(--foundry-test-args "${FOUNDRY_TEST_ARGS}")
  fi

  if [[ -n "${PROPERTIES_PATH}" ]]; then
    args+=(--properties-path "${PROPERTIES_PATH}")
  fi

  "${REPO_ROOT}/target/release/foundry-scfuzzbench" "${args[@]}"
done

printf 'scfuzzbench artifacts written under %s\n' "${BENCH_ROOT}"
