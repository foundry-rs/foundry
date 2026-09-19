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

## Cast run benchmarks

`foundry-cast-run-bench` measures `cast run` performance. It currently compares
ordinary `cast run` (`auto`, with block access list support) with `cast run --no-bal`
(`replay`) using the **same unmodified binary**. An HTTP proxy records RPC counts,
response bytes and request timings. `--include-miss` adds an optional arm that
locally injects JSON-RPC method-not-found for BAL requests; this measures local
fallback overhead without an unsupported provider's network round trip.

The supplied Cast must contain PR #16931 and advertise `--no-bal`. Both the runner
and build wrapper reject older binaries. BAL support is probed through the
provider for Cancun and later blocks, including pre-Amsterdam blocks; a successful
BAL response alone is not proof that Cast used it. The runner records the observed
path and retains
fallbacks, failures, timeouts and output differences.
Before Cancun, all arms replay without a BAL probe; the optional `miss` arm remains
a valid no-probe control.

### Capture and run

Use an archive HTTP endpoint that provides the selected blocks and parent state.
`--rpc-env VARIABLE_NAME` reads an existing environment variable containing its
URL; without it, the runner uses configured Foundry RPC settings, then the
repository's archive test endpoint. Keep credential-bearing URLs out of commands
and manifests. Providers without BAL support can measure replay and natural
fallback, but cannot establish BAL-hit performance.

Capture selects finalized block metadata before probing BAL availability. The
default panel has 12 nonempty blocks, with ordinary/large-block strata and
first/middle/last targets; coincident positions are deduplicated. Fix `--end-block`
and `--seed` to repeat a selection, or reuse the captured manifest directly:

```sh
cargo run --locked -p foundry-bench --bin foundry-cast-run-bench -- capture \
  --blocks 12 --candidate-blocks 120 --seed 7928 \
  --rpc-env BAL_BENCH_RPC_URL --output-dir /tmp/foundry-bal-panel

cargo run --locked -p foundry-bench --bin foundry-cast-run-bench -- run \
  --manifest /tmp/foundry-bal-panel/manifest.json \
  --cast /absolute/path/to/bal-capable/cast \
  --rpc-env BAL_BENCH_RPC_URL --rounds 10 --warmup-rounds 2 \
  --output-dir /tmp/foundry-bal-results
```

`--cast` is required. `--build-manifest /absolute/path/build.json` is optional and
verifies the binary against saved build provenance. Binary hash and version are
recorded even without a build manifest. `--include-miss` enables the third arm;
`--timeout-seconds` sets the whole-child deadline, not a BAL-specific deadline.
Arm order alternates between rounds; optional three-arm runs rotate all six
orders. Every attempt starts a new Cast process with disk storage caching
disabled. Validation compares full trace output, gas and status with replay
before measured samples check their output against the replay oracle.
The preflight rejects global execution/RPC overrides and records the selected non-secret
settings. It uses the runner’s configuration parser; comparisons across Cast versions
that change this configuration surface require reviewing that compatibility.

The report includes per-case auto/replay median time, paired time and RPC deltas,
request and byte counts, BAL response timing, and median/IQR/min/max distributions.
It emits a speedup only when every scheduled measured pair is present, unique,
complete and equivalent. Conditional BAL-hit timings do not replace overall auto
results. Warmup and validation attempts remain in raw artifacts but are excluded
from measured statistics.

Results retain `manifest.json`, `samples.jsonl`, `rpc-events.jsonl` and raw child
outputs. `summary.json` and `report.md` can be recomputed offline:

```sh
cargo run --locked -p foundry-bench --bin foundry-cast-run-bench -- report \
  --output-dir /tmp/foundry-bal-results
```

`common-results.json` projects valid completed samples into the existing common
schema. If none qualify, it is omitted and `summary.json` records
`no_common_projection`. [`bal-benchmark-v1.schema.json`](schema/bal-benchmark-v1.schema.json)
defines frozen panels and sample records; a run manifest wraps its panel with
execution provenance. [`bal-build-v1.schema.json`](schema/bal-build-v1.schema.json)
defines optional build provenance.

### Reproducible ref comparisons

Choose immutable base and candidate refs that both support `--no-bal`. An older
checkout or a pre-BAL base is unsuitable for this same-binary comparison. The
wrapper defaults to `origin/master` and `HEAD`, so explicitly select compatible
refs when either default is old:

```sh
BASE_REF='<bal-capable-base-sha>' CANDIDATE_REF='<bal-capable-candidate-sha>' \
  PANEL_MANIFEST=/tmp/foundry-bal-panel/manifest.json \
  RPC_ENV=BAL_BENCH_RPC_URL BENCH_ROOT=/tmp/foundry-bal-comparison \
  benches/scripts/pr-bal-bench.sh
```

The script builds one untouched Cast per distinct SHA in detached worktrees with
`cargo build --locked --profile profiling -p cast --bin cast`. It rejects source
mutations and differing Rust toolchains; identical refs reuse one binary.
`BAL_BENCH_FEATURES` supplies identical additional features. Set `BUILD_ONLY=1`
to stop after building; no panel or RPC access is required in that mode.

`base/build.json` and `candidate/build.json` retain source SHA, lockfile hash,
compiler, build arguments/environment, and Cast path/hash/version. The runner is
built from the current checkout; `runner-build.json` records its provenance and
local source hashes. All artifacts and worktrees survive success or failure;
`BENCH_ROOT` must be new. The script does not fetch or move refs.

Measured rounds alternate which ref runs first. `ROUNDS`, `WARMUP_ROUNDS`,
`TIMEOUT_SECONDS` and `INCLUDE_MISS=1` configure the run. `schedule.json` records
the frozen panel hash and execution order. Per-ref results live in
`results/{base,candidate}/{warmup,measured}-NNN`; each `aggregate` report combines
measured rounds and points back to original artifacts. Keep counts, endpoint,
features and deadlines identical; do not rerun only unfavorable samples.

RPC spans include network and service time and can overlap, so their sum is not
process wall time. Server caching and historical BAL generation are provider
properties; report them as unknown unless independently verified. Synthetic
fixtures and public RPC runs answer different questions. Neither a loopback
benchmark nor one provider's result establishes a universal Cast speedup.

### Tool correctness and synthetic coverage

The build wrapper tests use fake compiler tools and local Git repositories:

```sh
bash -n benches/scripts/pr-bal-bench.sh
python3 benches/scripts/test-pr-bal-bench.py -v
cargo test -p foundry-bench --bin foundry-cast-run-bench
```

The local Anvil harness uses recorded BAL responses to check first/middle/last
boundaries, repeated changes, system-operation ordering, pre-Amsterdam probing,
unsupported/unusable responses, and `--quick`/`--prestate-tracer` precedence:

```sh
python3 benches/scripts/test-bal-bench.py \
  --cast /absolute/path/to/bal-capable/cast \
  --anvil /absolute/path/anvil --runner /absolute/path/foundry-cast-run-bench \
  --include-miss --output-dir /tmp/foundry-bal-local-check
```

`--fixtures-only` runs direct Cast/replay checks without the benchmark runner.
`--build-manifest` is optional here too. Delayed responses have no assumed 500 ms
cutoff; incomplete BALs that produce divergent output must remain blocked from
performance comparisons. These are correctness checks, not real-provider speed
measurements. Ordinary Anvil mining alone does not provide live BALs.
Repeat with `--hardfork shanghai --include-miss` and a new output directory to
verify historical blocks replay without BAL probes across all three arms.

Measure the proxy's loopback overhead separately; do not subtract it from Cast
wall times:

```sh
cargo test -p foundry-bench --bin foundry-cast-run-bench \
  bal::proxy::tests::proxy_overhead -- --ignored --nocapture
```
