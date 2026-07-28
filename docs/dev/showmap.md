# Showmap corpus replay

`forge test --showmap-out <DIR>` replays the persisted fuzz/invariant corpus and
emits AFL-`afl-showmap`-style coverage files. Output is consumable by tools like
[`riesentoaster/differential-coverage`](https://github.com/riesentoaster/differential-coverage)
for cross-fuzzer / cross-approach coverage comparisons.

## Automatic corpus reuse

An ordinary `forge fuzz run` automatically assigns separate fuzz and invariant
corpus directories and uses coverage-guided corpus mutation. By default the
roots are `cache/fuzz/corpus` and `cache/invariant/corpus`; a customized
`failure_persist_dir` instead produces `<failure_persist_dir>/corpus`. Explicit
`corpus_dir` configuration or `--corpus-dir` overrides that location.
`forge test` only enables this behavior when a corpus directory is configured.

In invariant campaigns, Foundry also retains a worker-local dictionary of
argument-bearing top-level calls whose complete concrete execution won an edge
or hit-count coverage feature. It is bounded to 1,024 entries and 4 MiB of
calldata per worker. The retained startup replay seed has the same bound, so a
conservative campaign-level calldata bound is `(workers + 1) × 4 MiB`.

Each donor is keyed by the full hash of the canonical ABI function signature,
rather than only the four-byte selector. For each freshly generated invariant
call, Foundry first selects the destination and function normally. It may then
reuse calldata from a compatible donor, retarget it to that already-selected
destination, and mutate exactly one ABI argument. The generated call still
executes normally. Reusing a donor does not bypass the existing sequence-level
corpus admission rules: only sequence-level inputs that improve coverage or the
optimization objective are persisted.

Nested calls are not collected: their calldata alone does not reproduce the
caller, value, preceding state, or reentrancy context that made the original
execution interesting.

Whole-call donors are in-memory generation inputs, not additional corpus
entries. Corpus replay at startup reconstructs the donor dictionary from
coverage-winning calls in persisted sequences, then each invariant worker
learns locally. The existing
`dictionary_weight` controls how often a compatible donor is selected; zero
disables donor reuse. An effective `mutation_weight_abi = 0` disables both donor
collection and reuse.

This concrete mutation is most relevant to invariant campaigns where correlated
ABI arguments matter, including handlers that share a canonical signature. It
does not derive unknown cryptographic preimages or generally solve nonlinear
constraints, and does not claim a general hard-branch improvement. There is no
separate `forge fuzz seed` subcommand: `forge fuzz run` learns and reuses these
donors automatically, while the explicit CLI entry point for solver-assisted
pre-seeding is `forge test --symbolic-seed-corpus`.

## Campaign and inspection workflow

```bash
# 1. Populate or continue an explicitly located corpus with a normal campaign.
# Omitting --corpus-dir uses forge fuzz run's cache-backed default.
forge fuzz run --corpus-dir corpus --mc MyInvariantTest --mt invariant_

# 2. Decode and inspect the persisted sequences.
forge fuzz show corpus

# 3. Keep only entries that add coverage. The output path must not exist.
forge fuzz cmin corpus --corpus-out corpus-min \
  --mc MyInvariantTest --mt invariant_

# 4. Confirm that the minimized entries still replay for the selected test.
forge fuzz replay --corpus-dir corpus-min \
  --mc MyInvariantTest --mt invariant_

# 5. Export coverage for comparison.
forge fuzz run \
  --showmap-out coverage_data \
  --showmap-corpus-dir corpus-min \
  --showmap-approach foundry \
  --showmap-domain evm \
  --mc MyInvariantTest \
  --mt invariant_
```

Use the same test filters and replay-critical EVM/build options for the
campaign, minimization, replay, and showmap steps. `corpus_dir` may instead be
configured under the applicable `[profile.<name>.fuzz]` or
`[profile.<name>.invariant]` section in `foundry.toml`.

This skips the regular fuzz/invariant campaign and unit/table tests, then for
every selected fuzz/invariant test:

1. Resolves the per-test corpus dir (or `--showmap-corpus-dir <PATH>` override).
2. Walks every `worker*/corpus/*.json[.gz]` and deduplicates synchronized
   copies by corpus identity (UUID and timestamp).
3. Replays each entry through a fresh executor.
4. Aggregates per-call EVM instruction/PC hit maps and/or sancov edge bitmaps
   with saturating add.
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
