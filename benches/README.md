# Foundry Benchmarks

This directory contains performance benchmarks for Foundry commands across multiple repositories and Foundry versions.

## Prerequisites

Before running the benchmarks, ensure you have the following installed:

1. **Rust and Cargo** - Required for building the benchmark binary

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Foundryup** - The Foundry toolchain installer

   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

3. **Git** - For cloning benchmark repositories

4. [**Hyperfine**](https://github.com/sharkdp/hyperfine/blob/master/README.md) - The benchmarking tool used by foundry-bench

5. **Node.js and npm** - Some repositories require npm dependencies

## Running Benchmarks

Build and install the benchmark runner:

```bash
cargo build --release --bin foundry-bench
```

### Run CI benchmark suites locally

The canonical CI and nightly benchmark definitions live in
`benches/scripts/run-benchmark-suite.sh`. List the available profiles and suites
with:

```bash
benches/scripts/run-benchmark-suite.sh --list
```

| Profile | Suites run by CI |
| --- | --- |
| `ci` | `test`, `isolate`, `build`, `coverage` |
| `nightly` | `test`, `fuzz`, `build`, `coverage`, `symbolic` for both stable and nightly |

The `ci:symbolic` suite is also defined for local use but is currently disabled
in the regular workflow because the previous stable baseline does not support
symbolic execution.

After building the runner, invoke a suite from the repository root. These are
the same definitions used by CI, including pinned repositories, exclusions,
benchmark IDs, installation behavior, and output filenames:

```bash
# Run the regular CI test suite against a local Foundry build.
benches/scripts/run-benchmark-suite.sh ci test \
  --versions local \
  --output-dir ./benches

# Run the stable side of the nightly symbolic suite.
benches/scripts/run-benchmark-suite.sh nightly symbolic \
  --versions stable \
  --output-dir ./benches
```

Use `--dry-run` before the profile name to print the exact argument vector
without running the benchmark. Pass `--repos "org/repo[:rev][ <extra args>]"`
to replace a suite's repository list, matching the manual benchmark workflow's
override. An empty `--repos` value uses the suite default.

To install `foundry-bench` to your PATH:

```bash
cd benches && cargo install --path . --bin foundry-bench
```

#### Run with default settings

```bash
# Run all benchmarks on default repos with stable and nightly versions
foundry-bench --versions stable,nightly
```

#### Run with custom configurations

```bash
# Bench specific versions
foundry-bench --versions stable,nightly,v1.0.0

# Run on specific repositories. Default rev for the repo is "main"
foundry-bench --repos ithacaxyz/account,Vectorized/solady

# Test specific repository with custom revision
foundry-bench --repos ithacaxyz/account:main,Vectorized/solady:v0.0.123

# Run only specific benchmarks
foundry-bench --benchmarks forge_build_with_cache,forge_test

# Run only fuzz tests
foundry-bench --benchmarks forge_fuzz_test

# Run coverage benchmark
foundry-bench --benchmarks forge_coverage

# Run focused symbolic tests and report solver counters
foundry-bench --repos Vectorized/solady:v0.1.26,SorellaLabs/angstrom:73b55b8eca667b9a50fa4d8b6a7f45ec647420f5,farcasterxyz/contracts:3f37e21db8e9c6319b4a3d5f62b1c514ef01c36b --benchmarks forge_symbolic_test

# Combine options
foundry-bench \
  --versions stable,nightly \
  --repos ithacaxyz/account \
  --benchmarks forge_build_with_cache

# Force install Foundry versions
foundry-bench --force-install

# Verbose output to see hyperfine logs
foundry-bench --verbose

# Output to specific directory
foundry-bench --output-dir ./results --output-file LATEST_RESULTS.md
```

`forge_symbolic_test` keeps the existing focused commands for Solady, Angstrom, and Farcaster
and uses the generic symbolic command for other repositories.
Use `--symbolic-sidecar-output <FILE>` with exactly one `--versions` value to capture the
versioned per-run results. Like `--json-output`, the sidecar path is relative to `--output-dir`.

For filtered test discovery, use `--benchmarks forge_test_filtered` with test filters
in the repository's extra arguments or `foundry.toml`. This keeps the project's
dynamic-linking configuration and warms the selected tests twice. The first warmup
forces cleanup using the same root and configuration as the tests, then populates
normal artifacts. The second populates ABI discovery for that cache. Timed runs
retain that warm cache.
The result is labeled `Forge Test (Warm Cache)` because without effective filters
it measures ordinary unfiltered tests; it does not guarantee ABI-discovery coverage.
No unfiltered build runs during setup. For example:

```bash
foundry-bench --versions local --benchmarks forge_test_filtered \
  --repos "aave/aave-v4:1e8de8630dfeb26ad309d986eaec44c1ceb48a6d --match-contract EngineFlagsTest --match-test test_toBool_zero_returnsFalse --threads 1"
```

## Branch vs master PR-body workflow

Use this workflow when preparing performance numbers for a PR body. It keeps the
baseline and candidate builds isolated and makes the final table easy to audit.

### Command timing with foundry-bench

Use `foundry-bench` when the performance claim is about elapsed time for a
Foundry command on an existing Solidity repository. It answers questions like:

- Did this branch make `forge build` faster or slower?
- Did cached rebuilds change?
- Did `forge test` or `forge test --isolate` change?
- Did fuzz-test execution time change?
- Did `forge coverage --ir-minimum` change?
- Did focused symbolic test wall time or solver counters change?

Do not use `foundry-bench` for long-running invariant campaign quality,
throughput, corpus, showmap, or differential-coverage comparisons. Use
`foundry-scfuzzbench` for those.

Pick `BENCHMARKS` from the changed path:

| Changed path | Suggested `BENCHMARKS` |
| --- | --- |
| Compiler, artifact caching, project graph, dependency resolution | `forge_build_no_cache,forge_build_with_cache` |
| Test runner, EVM execution, cheatcodes, traces used by tests | `forge_test` |
| Isolation semantics or per-test state reset | `forge_isolate_test` |
| Fuzz runner execution overhead, fuzz fixtures, corpus replay behavior | `forge_fuzz_test` |
| Coverage instrumentation or report generation | `forge_coverage` |
| Symbolic executor or SMT solving | `forge_symbolic_test` |

Pick `REPOS` so the external project actually exercises the changed path. Use a
single focused repo first, then add more only if the result would otherwise be
unconvincing. The workflow accepts the same repo syntax as normal
`foundry-bench`: `org/repo[:rev][ <extra forge args>]`.

Examples:

```bash
# Test-runner or EVM execution change.
BENCHMARKS=forge_test
REPOS="ithacaxyz/account:v0.5.7"

# Build/cache change.
BENCHMARKS=forge_build_no_cache,forge_build_with_cache
REPOS="vectorized/solady:v0.1.26"

# Coverage change.
BENCHMARKS=forge_coverage
REPOS="uniswap/v4-core:46c6834698c48bc4a463a86d8420f4eb1d7f3b75"

# Symbolic change with focused counters.
BENCHMARKS=forge_symbolic_test
REPOS="Vectorized/solady:v0.1.26"
```

Run the matched branch-vs-base timing comparison:

```bash
BASE_REF=origin/master \
BENCHMARKS=forge_test \
REPOS="ithacaxyz/account:v0.5.7,vectorized/solady:v0.1.26" \
benches/scripts/pr-bench.sh
```

The script builds the `foundry-bench` harness from the current checkout once,
then points it at each checked-out workspace with
`FOUNDRY_BENCH_WORKSPACE_ROOT`. The `local` version builds and activates that
workspace with `FOUNDRY_BENCH_LOCAL_BUILD_PROFILE=profiling` before each run.
By default, the PR script sets `FOUNDRY_BENCH_LOCAL_BUILD_BINS=forge` so forge
benchmarks do not build unused targets. Set it to a comma- or whitespace-
separated list such as `forge,cast,anvil,chisel` when a run needs more binaries.
Keep `REPOS`, `BENCHMARKS`, and any per-repo extra arguments identical for
`master` and `candidate`. Override `CANDIDATE_REF`, `RUN_ID`, or `BENCH_ROOT`
when needed.

For PR bodies, reduce the two JSON/Markdown outputs into a concise table:

```md
### Results

| Benchmark | master | this PR | delta |
| --- | ---: | ---: | ---: |
| `forge_test/ithacaxyz-account` wall time | ... | ... | ... |
```

Include domain counters next to wall time when the benchmark produces them, for
example symbolic solver queries, reported solver time, invariant throughput, or
coverage relscore/relcov. If the delta is within noise, describe it as neutral
or inconclusive.

## Running scfuzzbench Campaigns

`foundry-scfuzzbench` runs a local scfuzzbench Foundry campaign, invokes the scfuzzbench
analysis/reporting pipeline, and copies stable artifacts into `<output-dir>/artifacts` for review by
humans or LLMs.

```bash
cargo run -p foundry-bench --bin foundry-scfuzzbench -- \
  --target-repo https://github.com/Recon-Fuzz/aave-v4-scfuzzbench.git \
  --target-ref v0.5.6-recon \
  --benchmark-type property \
  --timeout-seconds 60 \
  --workers 1 \
  --output-dir /tmp/foundry-scfuzzbench-aave \
  --foundry-bin "$(command -v forge)"
```

To compare a feature branch against `master` locally, run two matched campaigns:
one with a `forge` binary built from `master`, and one with a `forge` binary
built from the candidate branch. The runner intentionally records each campaign
separately; compare the resulting artifact bundles when drafting the PR body.

```bash
TARGET_REPO=https://github.com/Recon-Fuzz/aave-v4-scfuzzbench.git \
TARGET_REF=v0.5.6-recon \
BENCHMARK_TYPE=property \
TIMEOUT_SECONDS=3600 \
WORKERS=1 \
benches/scripts/pr-scfuzzbench.sh
```

Use the same `TARGET_REPO`, `TARGET_REF`, `BENCHMARK_TYPE`,
`TIMEOUT_SECONDS`, `WORKERS`, and `FOUNDRY_TEST_ARGS` for both runs. For
optimization campaigns, pass the same `PROPERTIES_PATH` to both runs. Summarize
the PR body from `llm_summary.md`, `REPORT.md`, `summary.csv`,
`cumulative.csv`, and the differential coverage artifacts in each output
directory. Override `BASE_REF`, `CANDIDATE_REF`, `RUN_ID`, or `BENCH_ROOT` when
needed.

By default the runner pins `https://github.com/tempoxyz/scfuzzbench.git@main`. Override that with
`--scfuzzbench-repo` and `--scfuzzbench-ref` while the scfuzzbench changes are being upstreamed.
Use `--foundry-bin` to benchmark an existing `forge`, `--foundry-ref` to build and benchmark a
Foundry ref from `--foundry-repo` (default `https://github.com/foundry-rs/foundry.git`), or neither
to use `forge` from `PATH`. The runner uses an isolated `HOME` so a selected `--foundry-bin` is not
shadowed by `~/.foundry/bin`. `--foundry-bin` must point to a file named `forge`; the runner resolves
`command -v forge` under the same campaign environment, verifies it is the selected binary, and
records the canonical path in `manifest.json`.

Optimization campaigns require `--properties-path`, which is passed to scfuzzbench as
`SCFUZZBENCH_PROPERTIES_PATH` and must be relative to the target repository. If GNU `timeout` or
GNU-style `sed -i` is unavailable, the runner installs local shims in the work directory and prepends
it to the campaign `PATH`. On platforms where `date -Is` is unavailable, it also installs a local
`date` shim for scfuzzbench log timestamps. If `local-run.sh` exits non-zero, the runner stops before
analysis so a failed setup or campaign cannot be reported as a successful artifact bundle.

The artifact bundle exposes:

- `REPORT.md`
- `events.csv`, `summary.csv`, `cumulative.csv`
- throughput/progress CSVs
- `showmap_campaign_manifest.json` and `showmap_campaigns/`
- `differential_coverage_relscores.csv`
- `differential_coverage_relcov.csv`
- runner resource and broken invariant reports
- optional `lcov-diff/` outputs when scfuzzbench produces coverage-diff files
- `llm_summary.md` and `manifest.json`, including the selected canonical `forge` path, optional
  Foundry repo/ref metadata, and optional `properties_path`

#### Command-line Options

- `--versions <VERSIONS>` - Comma-separated list of Foundry versions (default: stable,nightly)
- `--repos <REPOS>` - Comma-separated list of repos in org/repo[:rev] format
- `--benchmarks <BENCHMARKS>` - Comma-separated list of benchmarks to run
- `--force-install` - Force installation of Foundry versions
- `--verbose` - Show detailed benchmark output
- `--output-dir <DIR>` - Directory for output files (default: current working directory)
- `--output-file <FILE_NAME.md>` - Name of the Markdown output file. Defaults to `LATEST.md`
  unless `--json-output` is set, in which case Markdown is omitted unless explicitly requested
- `--common-json-output <FILE.json>` - Write results using the common benchmark schema in
  `benches/schema/benchmark-result-v1.schema.json`
- `--symbolic-sidecar-output <FILE.json>` - Write the opt-in v1 symbolic samples sidecar (requires exactly one version)

## Seeded Fuzzing Evaluation

`foundry-fuzz-eval` is a fast, repeatable scorer for fuzzer changes. It runs a fixed set of
targets once per seed, records which planted bugs each run found and, optionally, the source
coverage it reached, and reports means with 95% confidence intervals separately for a train and a
held-out test split. The default fixture suite runs 6 targets x 3 seeds, with coverage, in about
a second with a release `forge`, so an automated loop can call it hundreds of times. Use
`foundry-scfuzzbench` for the slower, realistic campaign signal.

```bash
cargo build --release --bin forge
cargo build --release -p foundry-bench --bin foundry-fuzz-eval

# Baseline.
target/release/foundry-fuzz-eval \
  --manifest benches/fixtures/fuzz-eval/manifest.toml \
  --forge-bin target/release/forge \
  --seeds 10 --coverage --label baseline \
  --output-dir /tmp/fuzz-eval/baseline

# Candidate: the same forge with an extra flag, or a forge built from another branch.
target/release/foundry-fuzz-eval \
  --manifest benches/fixtures/fuzz-eval/manifest.toml \
  --forge-bin target/release/forge \
  --seeds 10 --coverage --label candidate \
  --extra-forge-args "--invariant-depth 64" \
  --compare /tmp/fuzz-eval/baseline/results.json \
  --output-dir /tmp/fuzz-eval/candidate
```

Each run writes `results.json` (every target and seed: per-test status, bugs found, unexpected
failures, wall time, time to first failure, and LCOV counters), `SUMMARY.md`, and forge logs under
`logs/`. With `--compare`, it also writes `COMPARE.md` and ends with a verdict line. To compare two
existing results without running forge, pass `--compare <baseline.json> --current <current.json>`.
`--fail-unless-keep` exits with status 2 unless the verdict is `KEEP`.

Useful options:

- `--seeds N` runs seeds `1..=N`; `--seed-list 3,7,11` runs explicit seeds. Use the same seeds
  for baseline and candidate so deltas are paired.
- `--extra-forge-args "<args>"` is appended to every `forge test` and `forge coverage`
  invocation (split on whitespace, repeatable). This is how a treatment flag is evaluated without
  changing the harness. Do not pass `--fuzz-seed`; the harness sets it.
- `--fuzz-runs`, `--fuzz-timeout`, `--invariant-runs`, `--invariant-depth`, and
  `--invariant-timeout` override every target's budget. `--timeout-secs` overrides the
  per-invocation timeout.
- `--split train|test` and `--targets a,b` select a subset. `--cache-dir` sets where `repo`
  targets are cloned (default `<output-dir>/cache`).

### Manifest

Manifests are TOML. Local `path` values are relative to the manifest.

```toml
[defaults]
timeout_secs = 300             # per forge invocation
forge_args = []                # appended to forge test and forge coverage
coverage_args = []             # appended to forge coverage only
coverage_paths = ["src/"]      # source prefixes counted for coverage (default ["src/"])
env = { FOUNDRY_INVARIANT_SHRINK_RUN_LIMIT = "0" }

[defaults.budget]              # passed as FOUNDRY_FUZZ_RUNS, FOUNDRY_INVARIANT_DEPTH, ...
fuzz_runs = 256
invariant_runs = 8
invariant_depth = 8
# fuzz_timeout, invariant_timeout (seconds)

[[targets]]
name = "share-vault-rounding"
split = "test"                 # "train" or "test"
path = "project"               # local Foundry project, or `repo` + `ref` (+ optional `path` subdir)
match_contract = "ShareVaultInvariantTest"   # also match_test, match_path
expected_failures = ["invariant_depositRoundingIsBounded"]  # `test` or `Contract::test`
coverage_paths = ["src/ShareVault.sol"]
budget = { invariant_runs = 4, invariant_depth = 16 }
# forge_args, coverage_args, env, timeout_secs extend or override the defaults.
# failures_are_bugs = true counts every failing test as a found bug (for unenumerated targets).
# fresh_corpus = true gives every run an empty corpus directory (enables coverage-guided mode).
```

A run finds a bug when the matching expected test fails. Other failing tests are reported as
unexpected failures, which usually point at a harness or target problem. Every run gets fresh
fuzz and invariant failure-persistence directories so a counterexample found by one seed is never
replayed by another. Inherited `FOUNDRY_*` and `DAPP_*` variables are removed so the caller's shell
cannot change the configuration; set them through `env` instead. Budget variables are skipped when
the matching flag (for example `--fuzz-runs`) appears in the forge arguments, because forge lets
the environment take precedence over the flag.

`benches/fixtures/fuzz-eval/manifest.toml` is the default suite: one dependency-free Foundry
project with six planted bugs, four train and two test.

| Target | Split | Bug | Detector |
| --- | --- | --- | --- |
| `magic-lock` | train | backdoor behind two exact constants | stateless fuzz test |
| `fee-threshold` | train | off-by-one at a fee tier boundary | stateless fuzz test |
| `ether-bank-reentrancy` | train | balance written from a stale read after an external call | accounting invariant |
| `fee-config-access` | train | keeper path skips the caller check | only-authorized-changes invariant |
| `share-vault-rounding` | test | share rounding with no virtual shares (donation inflation) | bounded-loss invariant |
| `escrow-state-machine` | test | timeout claim accepted for a disputed deal | multi-step state-machine invariant |

Budgets are tuned per target so the default fuzzer finds each bug with roughly half of the seeds.
If a target reports no headroom for the baseline, lower its budget rather than removing it.
`manifest.scfuzzbench.toml` lists opt-in held-out targets from the scfuzzbench Recon-Fuzz
repositories; they need network access and take minutes per seed.

### Statistics and verdict

Seeds are the unit of replication. For each split and seed, the harness sums bugs found over the
split's targets (also reported as a fraction of expected bugs), averages branch and line coverage
over targets, and sums `forge test` wall time. `SUMMARY.md` reports the mean and a two-sided 95%
Student t-interval of those per-seed values, plus a per-target table and diagnostics:

- variance: per-target standard deviation across seeds; targets with a bugs-found std dev above
  0.4 or a branch coverage std dev above 5 percentage points are flagged as needing more seeds
- headroom: targets where every seed found every expected bug, every seed reached 100% branch
  coverage, or no seed found any expected bug
- problems: timed-out or failed forge runs, unexpected failing tests, and expected failures that
  matched no test

Timed-out or failed `forge test` runs count as finding no bugs, so a treatment that breaks or
slows forge is penalized; they are always listed under problems. With few seeds, t-intervals of
bounded counts can extend past the feasible range; add seeds for tighter intervals.

`--compare` computes `current - baseline` for each metric and split. When both runs used the same
seeds it uses a paired t-interval over per-seed differences, otherwise Welch's t-interval. A metric
counts as changed only when that interval excludes zero. A split improves when bugs found
increased, or when branch coverage increased and bugs found did not decrease. The verdict is:

- `KEEP`: the test split improved.
- `REVERT`: the test split regressed, or only the train split improved (suspected overfitting).
- `NEUTRAL`: neither split changed significantly, or only the train split regressed.

Tune changes against the train split and keep them only on a `KEEP` verdict. Time to first failure
is the reported duration of the failing test, so it includes setup and any shrinking; the fixture
manifest disables invariant shrinking. Coverage comes from a separate `forge coverage` campaign
with the same seed and budget, so it can differ from the `forge test` run.

## Benchmark Structure

- `forge_test` - Benchmarks non-isolated `forge test` command across repos
- `forge_build_no_cache` - Benchmarks `forge build` with clean cache
- `forge_build_with_cache` - Benchmarks `forge build` with existing cache
- `forge_fuzz_test` - Benchmarks non-isolated `forge test` with only fuzz tests (tests with parameters)
- `forge_coverage` - Benchmarks `forge coverage --ir-minimum` command across repos
- `forge_isolate_test` - Benchmarks isolated `forge test` command across repos
- `forge_symbolic_test` - Benchmarks `forge test --symbolic` for any repository, with focused recipes for Solady, Angstrom, and Farcaster. Compatibility JSON retains the median-wall-run counter projection; the optional sidecar records all five aligned samples, exact tests/statuses, and solver counters.

## Configuration

The benchmark binary uses command-line arguments to configure which repositories and versions to
test. Its generic defaults track the main branches of Account and Solady. Reproducible CI repository
pins and per-repository arguments are defined by `run-benchmark-suite.sh`; use `--list` to discover
those suites. You can override repositories using `--repos` with the format `org/repo[:rev]`.

## Results

Benchmark results are saved to `<output-dir>/LATEST.md` unless a custom output file is specified or
Markdown output is suppressed by `--json-output`. The report includes:

- Summary of versions and repositories tested
- Performance comparison tables for each benchmark type showing:
  - Mean execution time
  - Min/Max times
  - Standard deviation
  - Relative performance comparison between versions
- System information (OS, CPU cores)
- Detailed hyperfine benchmark results in JSON format

## ClickHouse Ingestion

Successful `master` versus branch runs from the Foundry Benchmarks workflow are ingested by
`.github/workflows/benchmarks-clickhouse.yml`. The ClickHouse schema and migrations are managed by
the benchmark infrastructure. Configure the `bench` GitHub environment with `CLICKHOUSE_HOST`,
`CLICKHOUSE_USER`, and `CLICKHOUSE_PASSWORD` secrets. The optional `CLICKHOUSE_DATABASE` and
`CLICKHOUSE_TABLE` variables default to `default` and `benchmark_results`. `CLICKHOUSE_HOST` may be
a bare host (using HTTPS on port 8443) or a full HTTPS endpoint. The ClickHouse user only needs
`INSERT` access to the destination table.

The ingester sends one `JSONEachRow` record per workflow attempt, comparison side, benchmark case,
and immutable workload commit. The versioned record includes raw timings, extensible counters,
source and workload provenance, runner metadata, and a deterministic `result_id`; infrastructure
owners use that contract to provision storage and release/trend views.

## Troubleshooting

1. **Foundry version not found**: Use `--force-install` flag or manually install with `foundryup --install <version>`
2. **Repository clone fails**: Check network connectivity and repository access
3. **Build failures**: Some repositories may have specific dependencies - check their README files
4. **Hyperfine not found**: Install hyperfine using the instructions in Prerequisites
5. **npm/Node.js errors**: Ensure Node.js and npm are installed for repositories that require them
