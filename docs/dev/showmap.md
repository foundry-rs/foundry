# Showmap corpus replay

`forge test --showmap-out <DIR>` replays the persisted fuzz/invariant corpus and
emits AFL-`afl-showmap`-style coverage files. Output is consumable by tools like
[`riesentoaster/differential-coverage`](https://github.com/riesentoaster/differential-coverage)
for cross-fuzzer / cross-approach coverage comparisons.

## Usage

```bash
# 1. Run a normal campaign with `corpus_dir` configured to populate the corpus.
forge test

# 2. Replay it.
forge test \
  --showmap-out coverage_data \
  --showmap-approach foundry \
  --showmap-domain evm
```

For a long stateless campaign, `forge fuzz run` can populate an empty or stale
corpus automatically before concrete fuzzing. The resolved campaign must have
at least 2,000,000 runs and no timeout below 15 minutes:

```bash
# Bootstrap at most one stale target, then run the requested concrete campaign.
forge fuzz run --match-test test_hard_branch \
  --runs 50000000 --timeout 900 --corpus-dir fuzz_corpus

# Inspect and replay the resulting corpus.
forge fuzz show fuzz_corpus
forge fuzz replay --match-test test_hard_branch --corpus-dir fuzz_corpus

# Measure aggregate EVM coverage without starting another campaign.
forge fuzz run --match-test test_hard_branch \
  --showmap-out coverage_data --showmap-approach auto-bootstrap \
  --showmap-domain evm --showmap-corpus-dir fuzz_corpus
```

Automatic bootstrap is limited to one eligible stale stateless target per Forge
process and to scalar ABI inputs without forks, FFI, or a persisted failure.
Passing symbolic candidates are kept only when concrete replay adds a new EVM
edge beyond a bounded filesystem sample of up to 256 existing and warmup corpus
entries; replay-confirmed failures must also fail the ordinary failure-replay
path and are never retained as coverage seeds. Inputs that reach `vm.sleep` or
interactive prompts are discarded by automatic warmup and replay. Use
`forge fuzz seed` only to force or inspect the bounded symbolic step
independently of a long campaign; unlike automatic mode, it retains every exact
branch-flipping replay.
See the [symbolic testing README](../../crates/evm/symbolic/README.md#automatic-fuzz-corpus-bootstrap)
for eligibility, solver-safety, and manifest details.

This skips the regular fuzz/invariant campaign and unit/table tests, then for
every selected fuzz/invariant test:

1. Resolves the per-test corpus dir (or `--showmap-corpus-dir <PATH>` override).
2. Walks `worker0/corpus/*.json[.gz]`.
3. Replays each entry through a fresh executor.
4. Aggregates per-call EVM (and/or sancov) edge bitmaps with saturating add.
5. Writes one or more files under `<showmap-out>/<approach>__<suite>__<test>/`.

## Flags

| Flag | Description |
|------|-------------|
| `--showmap-out <DIR>` | Output root. Required to enable showmap mode. |
| `--showmap-approach <NAME>` | Approach prefix; test identity is appended to form the dir name (default: `replay`). |
| `--showmap-trial <NAME>` | Trial id used as the filename (default: `trial-<unix_nanos>`, unique per invocation so reruns don't overwrite). |
| `--showmap-domain <evm\|sancov\|both>` | Bitmap(s) to dump (default: `evm`). |
| `--showmap-per-input` | Emit one file per corpus entry instead of one aggregated per test. |
| `--showmap-corpus-dir <PATH>` | Override the corpus dir to replay. |

## Output format

```
<showmap-out>/<approach>__<suite>__<test>/<trial>.txt              # aggregated
<showmap-out>/<approach>__<suite>__<test>/<trial>__<uuid>-<ts>.txt # --showmap-per-input
```

Each test gets its own approach dir so files inside it are trials of the same test,
which is the layout `differential-coverage` expects. `<suite>` is the full
`path/to/File.sol:Contract` identifier with `/`, `\`, and `:` replaced by `_`.

Each line: `<id>:<count>` where `count` is the saturating-summed raw hitcount.
Zero-hit edges are omitted. IDs are deterministic across `forge` processes:

| Domain | ID format | Meaning |
|--------|-----------|---------|
| `evm` | `evm_<bytecode_hash[:16hex]>_<pc:04x>` | The first 8 bytes of the keccak256 deployed-bytecode hash + the program counter that was hit. Source: line-coverage `HitMap`. |
| `sancov` | `sancov_0x<guard_idx:04x>` | Sancov guard index assigned at link time. |

The underscore separator (rather than `:`) between fields keeps the
`<id>:<count>` parser unambiguous.

## Differential-coverage workflow

To produce a campaign directory comparing approaches:

```bash
# Per-approach dirs are created automatically. Each invocation appends a new
# trial file; use --showmap-trial to set a stable id (e.g. across reruns).
forge test --showmap-out coverage_data --showmap-approach foundry --showmap-trial run_1
forge test --showmap-out coverage_data --showmap-approach foundry --showmap-trial run_2
# Other tools (echidna, medusa, …) write to the same `coverage_data/<name>/` layout.

# Optional: a "seeds-only" baseline produced by replaying just an initial corpus.
forge test --showmap-out coverage_data --showmap-approach seeds \
  --showmap-corpus-dir path/to/seeds_corpus

differential-coverage relcov coverage_data
differential-coverage relscore coverage_data
```

## Caveats

- `forge fuzz replay --corpus-dir <PATH>` replays corpus entries as seeds and
  reports whether they execute successfully for the selected targets. It is not
  the persisted-failure replay path. To reproduce the last saved fuzz failure,
  run `forge fuzz replay` without `--corpus-dir`.
- Unit and table tests are not runnable in showmap mode and are skipped.
- A test with no `corpus_dir` configured is `SKIP`ped with reason
  `"no corpus_dir configured for this test"`.
- A configured `corpus_dir` whose path does not exist on disk produces a `FAIL`
  with reason `"corpus directory not found: <path>"`.
- A `corpus_dir` that exists but is empty (or whose entries are all
  non-replayable for the current target) produces `(replay: 0 entries, 0 files)`
  and the test is `PASS`.
- `--showmap-domain sancov` (or `both`) on a build without sancov
  instrumentation produces no sancov lines; a warning is emitted.
- For invariant tests, txs are committed across the sequence, mirroring the
  campaign's stateful execution. For stateless fuzz tests, txs are not
  committed; only those matching the fuzzed function's selector are replayed.
- Coverage is aggregated across the whole replayed corpus per file. The output
  reflects coverage *reach*, not per-input contribution; use
  `--showmap-per-input` for the latter.
- EVM IDs intentionally include the deployed-bytecode hash. A target that
  deploys contracts with fuzz-dependent immutable values can therefore create
  different hash namespaces for otherwise identical code layouts. Before
  claiming a differential-coverage win, compare the namespace counts and
  confirm that the difference survives an appropriate source-level or
  program-counter diagnostic. Program-counter-only normalization can itself
  conflate unrelated contracts, so treat it as a diagnostic rather than a
  replacement coverage identity.
