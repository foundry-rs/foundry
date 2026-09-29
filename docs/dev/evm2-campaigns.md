# Forge campaign migration to evm2

[OSS-833](https://linear.app/tempoxyz/issue/OSS-833) owns fuzz and invariant campaigns,
including failure and corpus replay. The shared execution contract belongs to
[OSS-1030](https://linear.app/tempoxyz/issue/OSS-1030) and its implementation to
[OSS-823](https://linear.app/tempoxyz/issue/OSS-823). Campaign migration consumes that
boundary; it must not introduce a second owner for fork, snapshot, or inspector state.

## Starting point

At foundation commit `22faeddbf110284192635848d3d0cc5453d3c573`, the native
[`ethereum_runner/fuzz.rs`](../../crates/forge/src/ethereum_runner/fuzz.rs) runs isolated
stateless calls through a separate proptest loop. It uses empty fixtures, disables shrinking
and failure persistence, and rejects single-run replay. The native contract runner rejects
invariant tests. These paths demonstrate execution, not campaign parity.

Preserve the established algorithms in
[`executors/fuzz`](../../crates/evm/evm/src/executors/fuzz/mod.rs),
[`executors/invariant`](../../crates/evm/evm/src/executors/invariant/mod.rs), and the
[`shared campaign loop`](../../crates/evm/evm/src/executors/campaign.rs).
Port their execution dependencies after the shared session boundary is available, rather than
reimplementing the algorithms in the temporary native runner.

## Campaign requirements on the execution boundary

| Operation | Required behavior | Coordinating owner |
| --- | --- | --- |
| Start a case or invariant run | Restore post-setup accepted state, fork identity, environment and retained cheatcode state. Workers must not share mutable case state. | OSS-823, OSS-832; snapshot capsule decision OSS-1041 |
| Execute a stateless case | Return result and observations without changing the next case's baseline, including after `vm.assume`. | OSS-823, OSS-824 |
| Execute an invariant handler | Commit accepted sequence steps; preserve the existing rejection, revert, cancellation and environment-delay policies. | OSS-823; accepted-state decision OSS-1040 |
| Check an invariant | Observe the current sequence state without committing predicate writes into subsequent handler execution. Preserve assertion classification independently of EVM status. | OSS-825, OSS-824 |
| Generate the next call | Make fixtures, dictionary inputs, newly created contracts, target/exclusion rules and per-run cheatcode RNG available through their existing owners. | OSS-833 with OSS-823/824 |
| Shrink or replay | Start from the same baseline; preserve sender, target, calldata, value, delays, failure identity and full sequence semantics. Do not silently start a new campaign in replay mode. | OSS-833 with OSS-825 |
| Collect observations | Supply logs, traces, coverage, gas and state-derived dictionary feedback without exposing a general mutable REVM context. | OSS-834 with OSS-823 |

Before wiring the existing campaigns, agree on the concrete session entry points and the
observation/result contract with the foundation owner. The
[campaign boundary proposal](./evm2-campaign-boundary.md) recommends deferred acceptance and
separate test facts/observations as input to that design. In particular, commit acceptance must
not split evm2's accepted overlay from the backing database, and a case snapshot must include
environment and inspector state as well as account/storage data. Detached transaction writes
alone are not a snapshot.

## Parity fixtures

Run the same inputs against pinned master and candidate binaries, using separate projects and
cache/corpus directories. Record both commits, the evm2 dependency revision, compiler settings,
seed, worker count, runs, depth, exit status and failures. Use one worker for deterministic
baseline comparisons before adding multi-worker coverage. A successful compile or a passing
master test does not establish native support.

The first added CLI checks are
[`campaign_case_isolation` and `campaign_fork_case_isolation`](../../crates/forge/tests/cli/test_cmd/campaign.rs).
Both use [`FuzzCaseIsolation.t.sol`](../../testdata/fixtures/FuzzCaseIsolation.t.sol), fixed seed
`1`, and 16 accepted cases per test. They check ordinary storage, externally read storage,
block environment and persistent prank state after both accepted and assumption-rejected
cases. The fork variant seeds a local Anvil node, then reads its code and storage through a
pinned fork; no public RPC or credentials are required.

```sh
cargo build -p forge --bin forge
cargo test -p forge --test cli test_cmd::campaign::
```

Reuse the existing coverage below for the subsequent port. Each test remains a parity
requirement even when the foundation runner currently rejects the workflow.

| Requirement | Existing CLI coverage |
| --- | --- |
| Fixtures and generators | `does_not_evaluate_unused_fuzz_fixtures_for_unit_test_filter`, `fuzz_bounds_enum_inputs`, `invariant_fixtures` |
| Stateful sequencing and targets | `invariant_sequence_len`, `invariant_target_test_include_exclude_selectors`, `invariant_after_invariant` |
| Rejection and failure policy | `test_fuzz_fail_on_revert`, `should_not_fail_replay_assume`, `handler_vm_assert_global_flag_does_not_poison_invariant_checks` |
| Shrinking and failure replay | `handler_bug_replay_is_idempotent_after_shrink`, `handler_replay_uses_full_persisted_sequence_after_depth_decrease`, `forge_fuzz_replays_explicit_failure_file` |
| Seed and cheatcode RNG replay | `test_fuzz_run_replays_random_uint_failure`, `test_fuzz_run_replays_calldata_failure_after_rejects` |
| Corpus and showmap | `showmap_replay_emits_files`, `overloaded_fuzz_tests_use_distinct_paths`, `forge_fuzz_replay_scopes_generated_invariant_root_to_target` |

The existing tests live under [`test_cmd/fuzz.rs`](../../crates/forge/tests/cli/test_cmd/fuzz.rs),
[`test_cmd/invariant`](../../crates/forge/tests/cli/test_cmd/invariant/mod.rs), and
[`test_cmd/showmap.rs`](../../crates/forge/tests/cli/test_cmd/showmap.rs).
Extend stateful sequence coverage to a seeded local fork when that path is connected.

### Initial differential results

On 2026-09-29, the same CLI test executable was run beside each Forge binary: master
`7f3183c338a20a992f438d4a228d2c422ee365f0` and foundation
`22faeddbf110284192635848d3d0cc5453d3c573`. Both binaries were built locally in the debug
profile with Rust 1.98.0. The foundation pins evm2 to
`6eb1d262cd6049a9e709e517e97a0bea8877b9f9`. Each test creates its own isolated project.

| Check | Master | Foundation |
| --- | --- | --- |
| `campaign_case_isolation` | Pass | Pass |
| `campaign_fork_case_isolation` | Pass | Pass |
| `invariant_sequence_len` | Pass | Fails before producing the expected invariant sequence |
| `test_fuzz_run_replays_random_uint_failure` | Pass | Initial campaign fails with unsupported `vm.randomUint`; replay is not reached |
| `showmap_replay_emits_files` | Pass | Initial campaign rejects invariant tests; showmap is not reached |
| `showmap_replay_merges_unsynced_stateless_worker_corpora` | Pass | Silently runs a fresh 256-case campaign instead of reporting corpus replay |

The new isolation snapshots normalize timing and gas. They establish the asserted state and
successful-run-count behavior, not exact gas or generated-input parity. The four existing-test
failures above are pre-existing migration gaps, not regressions from the new fixtures. This
slice changes no runtime APIs, configuration, or persisted-data contracts; external consumer
compatibility remains part of the later implementation gate.

## Artifact compatibility

Replay master-produced failure and corpus artifacts on the candidate before generating new
candidate artifacts. Keep overload-specific paths, JSON/gzip formats, RNG metadata and failure
classification intact. Compare the actual replayed sequence and failure, not just exit codes.

For [showmap](./showmap.md), compare replay counts, file layout, and the `evm` coverage domain
using identical bytecode. Its IDs are bytecode-hash/program-counter based. Sancov IDs are
assigned at link time and cannot serve as cross-engine byte-for-byte parity evidence. Coordinate
coverage observations with OSS-834 rather than changing the persisted format in the campaign
port.
