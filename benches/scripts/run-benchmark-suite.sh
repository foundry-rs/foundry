#!/usr/bin/env bash
set -euo pipefail

join_repositories() {
  local IFS=,
  printf '%s' "$*"
}

CI_TEST_REPOSITORIES=$(join_repositories \
  "ithacaxyz/account:v0.5.7" \
  "vectorized/solady:v0.1.26 --nmc 'LifebuoyTest|LibBitTest|Base58Test|LibStringTest'" \
  "uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75 --nmc 'TickMathTestTest'" \
  "sparkdotfi/spark-psm:v1.0.0 --nmc PSMInvariants_TimeBasedRateSetting_WithTransfers_WithPocketSetting")

CI_ISOLATE_REPOSITORIES=$(join_repositories \
  "ithacaxyz/account:v0.5.7 --nmc SimulateExecuteTest" \
  "vectorized/solady:v0.1.26 --nmc 'SafeTransferLibTest|LifebuoyTest|LibBitTest|Base58Test|LibStringTest'" \
  "uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75 --nmc 'TickMathTestTest'" \
  "sparkdotfi/spark-psm:v1.0.0 --nmc PSMInvariants_TimeBasedRateSetting_WithTransfers_WithPocketSetting")

CI_BUILD_REPOSITORIES=$(join_repositories \
  "ithacaxyz/account:v0.5.7" \
  "vectorized/solady:v0.1.26" \
  "uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75" \
  "sparkdotfi/spark-psm:v1.0.0")

CI_COVERAGE_REPOSITORIES=$(join_repositories \
  "ithacaxyz/account:v0.5.7 --nmc SimulateExecuteTest" \
  "uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75" \
  "sparkdotfi/spark-psm:v1.0.0 --nmc PSMInvariants_TimeBasedRateSetting_WithTransfers_WithPocketSetting")

SYMBOLIC_REPOSITORIES=$(join_repositories \
  "Vectorized/solady:v0.1.26" \
  "SorellaLabs/angstrom:73b55b8eca667b9a50fa4d8b6a7f45ec647420f5" \
  "farcasterxyz/contracts:3f37e21db8e9c6319b4a3d5f62b1c514ef01c36b")

NIGHTLY_REPOSITORIES="aave/aave-v4:af1f0f2ba323ac6fbaaee3abf6be060c78e22d35"

SUITE_PROFILES=()
SUITE_NAMES=()
SUITE_BENCHMARKS=()
SUITE_REPOSITORIES=()
SUITE_MARKDOWN_OUTPUTS=()
SUITE_JSON_OUTPUTS=()
SUITE_FORCE_INSTALLS=()

define_suite() {
  SUITE_PROFILES+=("$1")
  SUITE_NAMES+=("$2")
  SUITE_BENCHMARKS+=("$3")
  SUITE_REPOSITORIES+=("$4")
  SUITE_MARKDOWN_OUTPUTS+=("$5")
  SUITE_JSON_OUTPUTS+=("$6")
  SUITE_FORCE_INSTALLS+=("$7")
}

# Regular CI suites. Symbolic is defined for local use but currently disabled in the workflow.
define_suite ci test \
  "forge_test,forge_fuzz_test" "${CI_TEST_REPOSITORIES}" \
  "forge_test_bench.md" "forge_test_bench.json" true
define_suite ci isolate \
  "forge_isolate_test" "${CI_ISOLATE_REPOSITORIES}" \
  "forge_isolate_test_bench.md" "forge_isolate_test_bench.json" false
define_suite ci build \
  "forge_build_no_cache,forge_build_with_cache" "${CI_BUILD_REPOSITORIES}" \
  "forge_build_bench.md" "forge_build_bench.json" false
define_suite ci coverage \
  "forge_coverage" "${CI_COVERAGE_REPOSITORIES}" \
  "forge_coverage_bench.md" "forge_coverage_bench.json" false
define_suite ci symbolic \
  "forge_symbolic_test" "${SYMBOLIC_REPOSITORIES}" \
  "forge_symbolic_bench.md" "" false

# Nightly suites run once for stable and once for nightly.
define_suite nightly test \
  "forge_test" "${NIGHTLY_REPOSITORIES}" \
  "" "{version}-{date}-forge_test.json" true
define_suite nightly fuzz \
  "forge_fuzz_test" "${NIGHTLY_REPOSITORIES}" \
  "" "{version}-{date}-forge_fuzz_test.json" false
define_suite nightly build \
  "forge_build_no_cache,forge_build_with_cache" "${NIGHTLY_REPOSITORIES}" \
  "" "{version}-{date}-forge_build.json" false
define_suite nightly coverage \
  "forge_coverage" "${NIGHTLY_REPOSITORIES}" \
  "" "{version}-{date}-forge_coverage.json" false
define_suite nightly symbolic \
  "forge_symbolic_test" "${SYMBOLIC_REPOSITORIES}" \
  "" "{version}-{date}-forge_symbolic.json" false

usage() {
  cat <<'EOF'
Usage:
  benches/scripts/run-benchmark-suite.sh [--dry-run] <profile> <suite> \
    --versions <versions> --output-dir <directory> [--repos <repositories>]
  benches/scripts/run-benchmark-suite.sh --list
EOF
}

list_suites() {
  local index
  for ((index = 0; index < ${#SUITE_PROFILES[@]}; index++)); do
    printf '%s:%s\n' "${SUITE_PROFILES[index]}" "${SUITE_NAMES[index]}"
    printf '  benchmarks: %s\n' "${SUITE_BENCHMARKS[index]}"
    printf '  repositories: %s\n' "${SUITE_REPOSITORIES[index]}"
  done
}

dry_run=false
if [[ ${1:-} == "--list" ]]; then
  list_suites
  exit 0
fi
if [[ ${1:-} == "--dry-run" ]]; then
  dry_run=true
  shift
fi
if [[ $# -lt 2 ]]; then
  usage >&2
  exit 2
fi

requested_profile=$1
requested_suite=$2
shift 2

versions=
output_dir=
repositories_override=
while [[ $# -gt 0 ]]; do
  case $1 in
    --versions)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      versions=$2
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      output_dir=$2
      shift 2
      ;;
    --repos)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      repositories_override=$2
      shift 2
      ;;
    *)
      printf 'error: unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z ${versions} || -z ${output_dir} ]]; then
  printf 'error: --versions and --output-dir are required\n' >&2
  usage >&2
  exit 2
fi

found=false
benchmarks=
repositories=
markdown_output=
json_output=
force_install=false
for ((index = 0; index < ${#SUITE_PROFILES[@]}; index++)); do
  if [[ ${SUITE_PROFILES[index]} == "${requested_profile}" && ${SUITE_NAMES[index]} == "${requested_suite}" ]]; then
    benchmarks=${SUITE_BENCHMARKS[index]}
    repositories=${SUITE_REPOSITORIES[index]}
    markdown_output=${SUITE_MARKDOWN_OUTPUTS[index]}
    json_output=${SUITE_JSON_OUTPUTS[index]}
    force_install=${SUITE_FORCE_INSTALLS[index]}
    found=true
    break
  fi
done

if [[ ${found} != true ]]; then
  printf 'error: unknown benchmark suite: %s:%s\n' "${requested_profile}" "${requested_suite}" >&2
  list_suites >&2
  exit 2
fi

if [[ -n ${repositories_override} ]]; then
  repositories=${repositories_override}
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "${script_dir}/../.." && pwd)
runner=${repo_root}/target/release/foundry-bench

args=("${runner}" --output-dir "${output_dir}")
if [[ ${force_install} == true ]]; then
  args+=(--force-install)
fi
args+=(
  --versions "${versions}"
  --repos "${repositories}"
  --benchmarks "${benchmarks}"
)
if [[ -n ${markdown_output} ]]; then
  args+=(--output-file "${markdown_output}")
fi
date=$(date -u +%Y-%m-%d)
if [[ -n ${json_output} ]]; then
  json_output=${json_output//\{version\}/${versions}}
  json_output=${json_output//\{date\}/${date}}
  args+=(--json-output "${json_output}")
  common_json_output=${json_output}
else
  common_json_output="${requested_profile}-${requested_suite}-${versions}-${date}.json"
fi
args+=(--common-json-output "common/${common_json_output}")
if [[ -n ${json_output} && ${requested_profile} == ci ]]; then
  manifest_output=${json_output%.*}-manifest.json
  args+=(
    --manifest-output "${manifest_output}"
    --suite "${requested_profile}:${requested_suite}"
  )
fi
args+=(--verbose)

if [[ ${dry_run} == true ]]; then
  printf 'argv:\n'
  printf '  %q\n' "${args[@]}"
  exit 0
fi

"${args[@]}"
