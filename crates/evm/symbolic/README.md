# foundry-evm-symbolic

`foundry-evm-symbolic` is Foundry's native symbolic EVM executor. It powers
`forge test --symbolic` and is intended to make symbolic tests feel like normal
Forge tests: write Solidity, run Forge, get either a proof result or a concrete
counterexample that is replayed through the normal Foundry executor before it is
reported.

Most users should interact with this crate through Forge. The Rust crate is the
engine that Forge calls after it has compiled contracts, run `setUp`, selected
tests, and prepared the concrete executor backend.

## Quick Start

Symbolic tests are Solidity functions named `check*` or `prove*`.

```solidity
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import "forge-std/Test.sol";

contract MathSymbolicTest is Test {
    function check_average(uint256 a, uint256 b) external pure {
        uint256 average = (a + b) / 2;

        // Forge should find an overflow counterexample.
        assertGe(average, a <= b ? a : b);
    }
}
```

Run it with:

```sh
forge test --symbolic --match-test check_average
```

Requirements:

- The configured solver must be available. The default solver command is `z3`.
  Install it locally with your package manager, for example `brew install z3`
  on macOS or `sudo apt-get install z3` on Ubuntu.
- `check*` and `prove*` tests are only selected when `--symbolic` is enabled
  and the contract is in a source path Forge compiles for the current project.
- A reported counterexample must replay concretely before Forge prints it as a
  failure.

`--match-test` filters function names or signatures. To filter by contract, use
`--match-contract`:

```sh
forge test --symbolic --match-test check_average
forge test --symbolic --match-contract MathSymbolicTest
```

## Result Semantics

Native symbolic testing is a preview feature. Results are scoped to the
executor's current EVM model and the configured exploration bounds.

Symbolic testing works best for Solidity-level properties that fit the modeled
EVM surface: arithmetic, storage, calldata, common call/reentrancy flows,
selected cheatcodes, and bounded stateful sequences. It finds and replays
concrete witnesses when it can. When a test depends on unsupported or unmodeled
behavior, Forge reports the run as incomplete instead of treating the property
as proven. It does not model full revm behavior, arbitrary unknown fork
accounts, or cryptographic preimage/collision search.

Forge reports symbolic test outcomes as:

- `PASS`: every explored path finished without a feasible failure under the
  currently modeled semantics and configured bounds.
- `FAIL` with a counterexample: the solver found a failing model and Forge
  replayed that concrete input or invariant sequence through the normal
  executor.
- `FAIL: incomplete symbolic execution (...)`: Forge could not complete the
  search or validate a counterexample for this run. Treat this outcome as "not
  established".

When `--json` is enabled, each symbolic test result includes a stable
`symbolic` object in addition to the legacy test fields. The schema lives at
`crates/evm/symbolic/assets/symbolic-result.schema.json` and records the
normalized status (`pass`, `fail_counterexample`, or `incomplete`), incomplete
reason kind, effective bounds, solver identity and counters, explicit
assumptions, call-trace location metadata, replay status, and counterexample
payload when one exists.

When Forge materializes a replay candidate, `symbolic.artifact` points to a
durable replay artifact written under the configured cache path. The artifact
schema lives at
`crates/evm/symbolic/assets/symbolic-counterexample.schema.json` and records
the replay status, bounds, assumptions, solver metadata, optional trace
reference, and concrete call data needed by downstream minimizers and exporters.

Symbolic execution can also seed coverage-guided fuzzing by concretizing
non-failing fuzz-test inputs into the configured `fuzz.corpus_dir`:

```sh
forge test --symbolic-seed-corpus --fuzz-corpus-dir fuzz_corpus
```

Forge symbolically executes matching fuzz tests, reuses their normal corpus
layout, and writes a successful concrete input as a seed for later fuzz runs.

Symbolic execution can import the same Foundry fuzz corpus as path-priority
hints for fuzz tests:

```sh
forge test --symbolic-use-fuzz-corpus --fuzz-corpus-dir fuzz_corpus
```

Imported corpus entries are bounded by `symbolic.corpus_seed_limit` and only
guide branch order; they do not prune feasible symbolic paths. JSON output
records the per-test corpus directory, import counts, and seed files that
matched a symbolic calldata variant under `symbolic.corpus_seeds.used`.

Fuzzing can also record branch frontier artifacts for later targeted symbolic
follow-up:

```sh
forge test --match-test test_hard_branch --fuzz-frontier-dir fuzz_frontiers
```

For example, a fuzz run may pass after reaching `feeMultiplier == 100` at a
`feeMultiplier < 100` guard; the frontier gives symbolic execution the replay
calldata and comparison site needed to solve the adjacent missed branch.

Forge writes one bounded artifact per fuzz test at
`<fuzz_frontier_dir>/<contract>/<test>/branch-frontiers.json`. The artifact
uses schema `foundry:fuzz.branch-frontiers@v1` and records the test signature,
configured record limit, and a `frontiers` array. Each frontier contains:

- a stable record index (`id`) within the artifact
- fuzz replay metadata (`seed`, `run`, `worker`) when available
- the concrete one-call sequence that reached the site
- the EVM comparison site (`address`, `pc`, `opcode`, `opcode_name`)
- concrete operands (`lhs`, `rhs`), the comparison result, and an
  `operand_delta` priority score interpreted according to opcode signedness
- whether the call also expanded the worker's coverage map (`new_coverage`),
  present only when edge coverage is collected via a corpus directory, edge
  coverage metrics, or sancov, and omitted otherwise

Manual frontier capture is opt-in and bounded by `fuzz.frontier_limit` (default
256). The automatic bootstrap described below enables a temporary bounded
capture without changing project configuration. Capture reuses the fuzzer's
comparison-operand inspector and does not store traces.

Symbolic execution can consume those artifacts to solve the opposite side of
captured comparisons and write replay-confirmed inputs into the fuzz corpus:

```sh
forge test --match-test test_hard_branch \
  --fuzz-frontier-dir fuzz_frontiers \
  --fuzz-corpus-dir fuzz_corpus \
  --symbolic-use-fuzz-frontiers
```

Forge imports up to `symbolic.frontier_limit` records (default 256), replays the
recorded one-call seed as a path-priority hint, constrains symbolic execution to
flip the captured comparison result, and persists only candidates that replay
with the expected concrete outcome.

To focus solver time on specific captured sites, pass frontier artifact IDs,
comparison PCs, or calldata selectors:

```sh
forge test --match-test test_hard_branch \
  --fuzz-frontier-dir fuzz_frontiers \
  --fuzz-corpus-dir fuzz_corpus \
  --symbolic-use-fuzz-frontiers \
  --symbolic-frontier-ids 3,7 \
  --symbolic-frontier-pcs 128,412 \
  --symbolic-frontier-selectors 0x12345678
```

`symbolic.frontier_ids`, `symbolic.frontier_pcs`, and
`symbolic.frontier_selectors` default to `[]`, meaning any value for that
dimension. Non-empty filters compose conjunctively, so the example imports only
records matching one of the requested IDs, one of the requested PCs, and one of
the requested selectors. Forge keeps the artifact order as the priority order
after filtering, imports up to `symbolic.frontier_limit` records, reports how
many records were imported or skipped by target filters, and warns if a
requested target cannot be imported.

## Automatic Fuzz Corpus Bootstrap

`forge fuzz run` automatically gives an empty or stale stateless corpus one
small symbolic bootstrap before an effectively long concrete campaign:

```sh
forge fuzz run --runs 50000000 --timeout 900 \
  --match-test test_hard_branch
```

There is no routine hybrid flag. Forge enables this prelude only when the
resolved per-test campaign has at least 2,000,000 runs and has no timeout below
15 minutes. Fuzzing stops at the first active bound, so a 15-minute timeout with
the default 256 runs, or two million runs with a short timeout, does not qualify.
Automatic targets must use only scalar ABI inputs (`bool`, integer, address, or
fixed bytes). Dynamic/composite inputs, function pointers, invariant tests,
fork-backed or FFI-enabled tests, tests with a persisted fuzz failure,
replay/showmap/list/rerun modes, gas reports, and ordinary `forge test` runs are
excluded. Clear an obsolete failure file, or use the explicit command below,
before asking Forge to bootstrap that target again.

For one stale matching target per Forge process, bootstrap runs 16 ordinary
concrete cases, capped at five seconds and 256 rejects, while retaining up to 32
comparison frontiers. It stages at most 256 existing entries into an isolated
temporary warmup corpus, so the prelude never asks the normal corpus loader to
scan the complete on-disk corpus. Each frontier gets at most one second of
symbolic time, 256 paths, and 1,024 solver queries. Forge checks configured
built-in portfolio entries in order and uses the first available
memory-bounded solver without racing them. Automatic mode currently supports Z3
and Bitwuzla, injects their native 256 MB memory limit, and skips other solver
backends rather than running them without a hard memory bound.
Project-configured solver commands, custom solver paths, and custom portfolio
entries are not executed automatically; use the explicit command below after
reviewing trusted solver settings. Parallel suites share the same process-wide
symbolic slot and do not receive additional budgets. When several eligible
stale tests match, runner scheduling determines which one claims the slot; use
`--match-test` or `--match-contract` to select a bootstrap target deliberately.

Every candidate is replayed through the normal concrete executor and must flip
the exact address, program counter, opcode, and comparison result that produced
its frontier. Replays discarded by `vm.assume` or `vm.skip` are rejected.
Automatic warmup and replay executors also discard inputs that reach
host-blocking `vm.sleep` or interactive `vm.prompt*` cheatcodes; explicit
symbolic commands keep their existing cheatcode behavior.
Passing candidates are persisted only when a second concrete replay contributes
a new EVM edge beyond a baseline of up to 256 sampled existing and warmup corpus
entries. The filesystem sample is bounded rather than exhaustive or
order-stable. A replay-confirmed failure must also fail the ordinary
persisted-counterexample replay path before it is reported. Failure candidates
are not written to the coverage corpus or completion manifest; a confirmed one
uses the normal persisted failure file, while a replay mismatch is discarded
and remains retryable. Automatic passing candidates use an in-run dedupe set and
a direct write, avoiding a full-corpus dedupe scan per frontier. This coverage
gate filters branch-flipping candidates that do not improve the sampled corpus
without making prelude cost grow with an arbitrarily large corpus.

After a completed non-failing attempt, Forge writes an atomic target-scoped
manifest beside the target corpus. Its fingerprint covers the bootstrap
version, full test identity, compiled test bytecode set, EVM mode, replay
semantics, and normalized
symbolic configuration, including the effective sender and EVM environment. An
unchanged target skips bootstrap on later runs. A missing, corrupt, or
mismatched manifest is stale; clearing a corpus that previously contained
entries also makes it stale. A completed bounded reject/incomplete or negative
attempt remains current so unsuitable targets do not pay the cost on every
campaign. Missing solvers and interrupted attempts remain retryable and do not
stop concrete fuzzing.

When `--corpus-dir` is omitted, `forge fuzz run` uses the same automatic corpus
root on every invocation: `cache/fuzz/corpus`, or `<fuzz.failure_persist_dir>/corpus`
when a custom failure directory is configured. The concrete campaign starts
after bootstrap and keeps its original run and timeout bounds.

### Force or inspect a bootstrap

`forge fuzz seed` remains an advanced, explicit corpus-building command. It is
useful when evaluating a target, preserving frontier artifacts, or forcing a
retry outside a long campaign, including supported non-scalar targets excluded
from automatic mode:

```sh
forge fuzz seed --match-test test_hard_branch --corpus-dir fuzz_corpus
forge fuzz run --match-test test_hard_branch --runs 50000000 \
  --timeout 900 --corpus-dir fuzz_corpus
```

The explicit command runs 16 concrete cases by default and tries up to 32
frontiers at one second each. `--runs`, `--frontier-limit`, and
`--solver-timeout` override those diagnostic bounds. It excludes invariants and
exits without starting the subsequent campaign. Unlike automatic mode, it
persists every exact branch-flipping replay, so measure its output before
assigning value to candidate count alone.

Useful follow-up commands are:

```sh
forge fuzz show fuzz_corpus
forge fuzz replay --match-test test_hard_branch --corpus-dir fuzz_corpus
forge fuzz run --match-test test_hard_branch \
  --showmap-out coverage_data --showmap-corpus-dir fuzz_corpus
```

This workflow is intended for cold, one-call branches visible as EVM
comparisons, such as arithmetic bounds or threshold checks. Paired cold-corpus
trials on unchanged Recon targets produced strict aggregate showmap supersets in
five current-default, stable-bytecode trials: two Nerite liquidation trials
gained 44 EVM IDs in existing bytecode namespaces (30 additional program
counters in a cross-namespace diagnostic), and three Aave v4 liquidation-bonus
trials gained 9 IDs and 9 program counters each. Three earlier Nerite trials at
a 256-frontier development budget also produced strict stable-namespace gains,
but are not counted as evidence for the final 32-frontier default. The added
inputs were boundary values for `forge-std` `bound`; this is repeatable
cold-corpus evidence, not a protocol bug or a broad DeFi time-to-bug
improvement.

Three Liquity cold-corpus pairs initially appeared to gain 1,458 to 1,582 IDs,
but an audit found that nearly all of that delta came from new bytecode-hash
namespaces when fuzz inputs became constructor immutables. Only 6 to 32 global
program-counter offsets remained in a diagnostic that can itself conflate
contracts, so those raw deltas are not counted as positive evidence. A separate
one-hour automatic Liquity pair reached 18.08 million concrete control runs and
18.26 million automatic runs. Its 578 ms prelude persisted one input that added
an edge beyond the 16-run warmup, but both campaigns ended tied at 5 live edges,
7 features, and 2,755 program-counter offsets. This validates bounded activation
and replay filtering, not a lasting coverage or time-to-bug win.

Final 15-minute automatic/control pairs also delimit the current claim. On the
Nerite target, a 1.4-second prelude persisted three inputs; the concrete
campaign then reached two additional program-counter offsets by five seconds,
while the control reached the same offsets by eight seconds of concrete
fuzzing. The automatic campaign finished with 14 corpus entries and the control
with 12, but
both final showmaps contained the same 27,090 EVM IDs and 17,549 global
program-counter offsets. On the Aave v4 liquidation-bonus target, a 496 ms
prelude persisted one of two replayed candidates and retained more inputs that
individually covered the cold path, but both final showmaps contained the same
1,119 EVM IDs. Automatic mode ran about 3% and 2.1% fewer concrete cases,
respectively. Nerite added 96 MiB of maximum resident memory; Aave showed no
memory penalty. These pairs support bounded corpus diversification and, in the
Nerite case, modest early cold-path reach. They do not establish a lasting
coverage or time-to-bug advantage.

An uncapped development run on Solady exposed a transient Z3 child-process
maximum resident set above 2 GiB. The final automatic configuration adds the
solver-native 256 MB limit described above; a focused repeat peaked at 327 MB
in the measured automatic command. Explicit symbolic commands remain governed
by their configured solver command and are not changed by this automatic-only
limit.

Candidate generation alone is not evidence of value: an unchanged Solady WETH
trial replay-confirmed seven explicit candidates but added no showmap IDs. The
automatic new-edge gate is designed around that negative control. Earlier long
automatic-assist campaigns on Aave, Spark, Tomb, and Solady targets also did not
produce useful candidates. Nonlinear arithmetic, hashes, signatures,
fork-heavy setup, invariant state machines, and deep unsupported call paths
remain poor fits.

> **Hash-model caveat:** `PASS` also assumes collision and preimage resistance
> for symbolic `KECCAK256` and hash-like precompile terms. The executor may use
> equal symbolic hashes to infer equal symbolic preimages or lengths in modeled
> cases, and it does not search for Keccak collisions or adversarial preimages.
> Concrete counterexamples are still replayed before they are reported.

Symbolic exploration is bounded by configuration, including
`symbolic.max_depth`, `symbolic.max_paths`, `symbolic.max_solver_queries`,
dynamic calldata length settings, and `symbolic.invariant_depth`.

`Incomplete` can occur when exploration reaches a configured bound, the solver
times out or returns `unknown`, a test uses unsupported EVM or cheatcode
semantics, a backend error occurs, or a solver model does not replay concretely.
When a solver candidate does not replay, it can still be shown as a diagnostic
legacy top-level `counterexample`; treat it as confirmed only when
`symbolic.status` is `fail_counterexample` and `symbolic.replay.status` is
`confirmed`.

Current modeling notes:

- Unsupported opcodes, world-state behavior, or cheatcode forms are reported as
  incomplete results with an explanatory reason.
- Symbolic `KECCAK256` supports common Solidity storage patterns; arbitrary
  symbolic hashing may require heuristics and can make a run incomplete.
- `SELFDESTRUCT` follows the active fork. Before Cancun it deletes the account;
  from Cancun onward it only deletes contracts created in the same top-level
  symbolic transaction, otherwise it transfers balance and halts while
  preserving code and storage. Cancun beneficiaries must resolve to concrete
  addresses; unresolved symbolic beneficiaries report incomplete.
- Counterexamples are shown only after successful concrete replay.

## Writing Symbolic Tests

Stateless symbolic tests use ordinary ABI parameters. The executor creates
symbolic calldata from the function ABI and explores feasible EVM paths.

```solidity
contract RiddleTest is Test {
    function check_riddle(uint256 x) external pure {
        uint256 sender = uint160(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);

        unchecked {
            require(x * x < sender);
        }

        require(x > sender);
        require(x & 0x800 != 0);
        require(x & 0x10000 == 0);

        assert(false);
    }
}
```

In this style:

- `require(...)` prunes paths when the condition is false.
- `vm.assume(...)` also prunes paths.
- `assert`, forge-std assertions, and DSTest failure signals are treated as
  properties to disprove.
- User reverts terminate the current path. If every path reverts, Forge reports
  a revert-all result instead of a proof.

Dynamic ABI inputs are bounded. Use `forge-config:` inline annotations or
`foundry.toml` to choose lengths.

```solidity
contract BytesSymbolicTest is Test {
    /// forge-config: default.symbolic.array_lengths = [3]
    function check_bytes(bytes memory data) external pure {
        require(data.length == 3);
        if (data[0] == 0xaa && data[1] == 0xbb && data[2] == 0xcc) {
            assert(false);
        }
    }
}
```

Dynamic leaves are traversed in deterministic ABI pre-order. Lengths resolve in
this order:

1. `symbolic.dynamic_lengths`, keyed by ABI argument name or generated symbolic
   name such as `calldata_0`.
2. `symbolic.default_array_lengths` for arrays, or
   `symbolic.default_bytes_lengths` for `bytes` and `string`.
3. The legacy positional `symbolic.array_lengths`, applied to the next dynamic
   leaf that was not matched by a named or type-specific default.
4. `symbolic.default_dynamic_length`.

Length-set config fields accept Halmos-style arrays and expand into separate
symbolic calldata shapes. For nested dynamic values, Foundry explores the cross
product implied by the selected outer lengths. Eager calldata expansion is capped
by the effective symbolic path width (`symbolic.width` / `symbolic.max_paths`).
Extra positional `array_lengths` entries are rejected as config errors.

Pending symbolic paths are explored in breadth-first order by default. Set
`symbolic.exploration_order = "dfs"` to use depth-first ordering instead. This
only changes which queued path is explored next; it does not change path limits,
solver query limits, or solver portfolio scheduling.

Supported ABI shapes include:

- integers, booleans, addresses, fixed bytes, and dynamic bytes
- strings, constrained to printable ASCII for model extraction
- fixed and dynamic arrays
- tuples and structs, represented through ABI tuples

## Stateful Symbolic Invariants

When `--symbolic` is enabled, `invariant*` and `statefulFuzz*` functions use a
bounded symbolic call sequence instead of the normal invariant fuzzer.

```solidity
contract CounterInvariant is Test {
    Counter counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    /// forge-config: default.symbolic.invariant_depth = 4
    function invariant_counterNeverFive() public view {
        assertTrue(counter.value() != 5);
    }
}
```

Forge reuses its invariant target discovery for target contracts, selectors, and
senders. The symbolic executor chooses a bounded sequence from that discovered
set, generates symbolic arguments with the same ABI model used for stateless
tests, preserves symbolic world state between calls, and replays a concrete
sequence before reporting a counterexample.

Some invariant harnesses deploy dependency contracts in `setUp`, then rely on
those dependencies having satisfiable environment state during the campaign. For
example, a lending invariant may call an ERC20 mock for balances and allowances,
or an oracle mock for a price, without exposing target functions that write
those values first. In that case the dependency storage remains the concrete
state produced by `setUp`, and symbolic execution can only explore paths
reachable from that concrete dependency state.

Use Foundry's existing `vm.setArbitraryStorage(address)` cheatcode to mark those
environment dependency contracts as symbolic storage targets:

```solidity
function setUp() public {
    loanToken = new ERC20Mock();
    collateralToken = new ERC20Mock();
    oracle = new OracleMock();

    vm.setArbitraryStorage(address(loanToken));
    vm.setArbitraryStorage(address(collateralToken));
    vm.setArbitraryStorage(address(oracle));

    targetContract(address(this));
}
```

If the harness imports a smaller Hevm interface that does not expose this
cheatcode, declare a local interface with `setArbitraryStorage(address)` and
cast it to `address(vm)`.

Keep this scoped to external dependencies that model the environment, such as
token balances, token allowances, and oracle prices. Do not blanket-mark the
invariant harness or protocol state as arbitrary; that can create unreachable
states and counterexamples that concrete replay rejects.

Replay currently materializes only concrete storage slots observed during
symbolic execution. Dependency storage keyed by symbolic calldata is still
explored symbolically, but it can replay-filter to `Incomplete` or `mismatch`
instead of a confirmed counterexample if the required slot is not concrete.

## Configuration

The primary configuration path is native Foundry config.

```toml
[profile.default.symbolic]
solver = "z3"
# Optional exact command. When set, this overrides `solver`.
# solver_command = "z3 -in -smt2"
# Optional solver names or commands to race in parallel. Ignored when
# `solver_command` is set. Entries with spaces/quotes/backslashes are parsed as
# argv strings, not shell snippets.
# solver_portfolio = ["yices", "z3"]
timeout = 30
max_depth = 10000
max_paths = 1024
exploration_order = "bfs"
max_solver_queries = 10000
default_dynamic_length = 2
max_dynamic_length = 256
array_lengths = []
dynamic_lengths = {}
default_array_lengths = []
default_bytes_lengths = []
max_calldata_bytes = 4096
invariant_depth = 10
frontier_ids = []
frontier_pcs = []
frontier_selectors = []
symbolic_call_targets = false
dump_smt = false
storage_layout = "solidity"
```

The same values can be set inline with NatSpec:

```solidity
/// forge-config: default.symbolic.timeout = 120
/// forge-config: default.symbolic.array_lengths = [2, 4]
/// forge-config: default.symbolic.dynamic_lengths = { data = [3] }
/// forge-config: default.symbolic.default_bytes_lengths = [8]
/// forge-config: default.symbolic.exploration_order = "dfs"
/// forge-config: default.symbolic.invariant_depth = 6
function check_with_bounds(bytes memory data, uint256[] memory b) external {
    // ...
}
```

Common CLI and environment overrides:

```sh
forge test --symbolic
forge test --symbolic --symbolic-solver yices
forge test --symbolic --symbolic-solver cvc5
forge test --symbolic --symbolic-solver bitwuzla
forge test --symbolic --symbolic-solver-command "z3 -in -smt2"
forge test --symbolic --symbolic-solver-portfolio yices,z3
forge test --symbolic --symbolic-timeout 120
forge test --symbolic --symbolic-array-lengths 2,4
forge test --symbolic --symbolic-invariant-depth 6
forge test --symbolic --symbolic-call-targets
forge test --symbolic --symbolic-dump-smt

FOUNDRY_SYMBOLIC=true forge test
FOUNDRY_SYMBOLIC_SOLVER=z3 forge test --symbolic
FOUNDRY_SYMBOLIC_SOLVER_COMMAND="z3 -in -smt2" forge test --symbolic
FOUNDRY_SYMBOLIC_SOLVER_PORTFOLIO="yices,z3" forge test --symbolic
FOUNDRY_SYMBOLIC_TIMEOUT=120 forge test --symbolic
```

Known solver names are `z3`, `yices`, `cvc5`, `cvc5-int`, `bitwuzla`, and
`bitwuzla-abs`. Unknown `symbolic.solver` values are treated as z3-compatible
executables and are invoked with `-in -smt2` to preserve the old
`symbolic.solver = "/path/to/z3"` behavior. Use `symbolic.solver_command` for
non-z3-compatible command lines or wrapper tools.

`symbolic.solver_portfolio` runs solvers in configured order with staged starts:
the first entry starts immediately, the second starts shortly after if the query
is still unresolved, and later entries are treated as rescue solvers. If a solver
finishes without a decisive result and no other solver is running, the next
pending entry starts immediately. The first `sat` response wins after its model
is validated for model-producing queries. `unsat` responses are used only after
all configured solvers that were needed to rule out `sat` have finished, and
`unknown` results only win if no configured solver returns a decisive response.
A nonempty `symbolic.solver_command` overrides both
`symbolic.solver_portfolio` and `symbolic.solver`; otherwise a nonempty
portfolio overrides `symbolic.solver`. Portfolio entries without whitespace,
quotes, or backslashes are resolved like `symbolic.solver` values. Entries with
whitespace, quotes, or backslashes are split into argv parts like
`symbolic.solver_command`; they are not executed through a shell.
For latency-sensitive local runs, start with a small portfolio such as
`["yices", "z3"]`. Broader portfolios can help on solver-diverse workloads but
can still use more CPU and be slower when one fast solver already handles most
queries. `--symbolic-dump-smt` prints each solver's configured order and launch
delay with the per-query portfolio outcomes so solver mixes can be compared
without changing execution semantics. It also prints an aggregate portfolio
summary at the end of the run, for example:

```text
--- symbolic solver portfolio summary ---
queries: 48
solver runs: 64
rescue solver runs: 0
not-started solver runs: 32
non-primary wins: 0
rescue wins: 0
cancelled after winner: 0
invalid models: 0
solver errors: 0
winner counts:
  yices-smt2 --bvconst-in-decimal: 48
launch counts:
  yices-smt2 --bvconst-in-decimal: 48
  z3 -in -smt2: 16
outcome counts:
  not-started: 32
  sat-valid: 32
  unsat: 32
```

Forge warns when a configured portfolio is degraded because one or more solver
entries are not available, but it still uses the entries that can be invoked.

Security note: `symbolic.solver_command`, custom `symbolic.solver` values, and
custom or command-like `symbolic.solver_portfolio` entries execute local programs
when symbolic tests run. This also applies when these values come from inline
`forge-config:` or translated legacy `@custom:halmos` annotations. Review solver
settings before running symbolic tests from untrusted projects.
Timeouts and portfolio cancellation terminate only the direct solver child
process. Wrapper commands should forward termination to any subprocesses they
spawn so descendant solvers do not outlive the cancelled query.

Halmos-style annotations are accepted as compatibility input and translated into
the same internal config:

```solidity
/// @custom:halmos --array-lengths 2,4 --width 32 --depth 256
function check_legacy(bytes memory a, bytes memory b) external {
    // ...
}
```

Supported compatibility flags are:

- `--array-lengths`
- `--loop`
- `--width`
- `--depth`
- `--invariant-depth`
- `--solver-timeout`
- `--solver-timeout-branching`
- `--solver-timeout-assertion`
- `--solver`
- `--solver-command`

Native `forge-config:` values win when both native and compatibility annotations
set the same symbolic option.

## SVM Compatibility Helpers

The executor recognizes a Halmos-style symbolic VM helper address:

```solidity
address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);

interface Svm {
    function createUint256(string calldata name) external returns (uint256);
    function createInt256(string calldata name) external returns (int256);
    function createBytes32(string calldata name) external returns (bytes32);
    function createAddress(string calldata name) external returns (address);
    function createBool(string calldata name) external returns (bool);
    function createBytes(string calldata name) external returns (bytes memory);
    function createBytes(uint256 length, string calldata name) external returns (bytes memory);
    function createString(string calldata name) external returns (string memory);
    function createString(uint256 length, string calldata name) external returns (string memory);
    function createBytes4(string calldata name) external returns (bytes4);
    function createCalldata(string calldata name) external returns (bytes memory);
    function enableSymbolicStorage(address target) external;
    function setArbitraryStorage(address target) external;
    function snapshotStorage(address target) external returns (uint256);
    function snapshotState() external returns (uint256);
}
```

Forge also treats several `vm.random*` cheatcodes as symbolic value creators when
running symbolically. Dynamic byte and string creators use the same dynamic ABI
bounds as function arguments.

## How It Works

Forge drives the symbolic executor in these stages:

1. Compile and deploy contracts with the normal Foundry flow.
2. Execute `setUp` concretely, including fork-backed setup when the Forge
   executor is forked.
3. Classify `check*` and `prove*` functions as stateless symbolic tests, and
   `invariant*` or `statefulFuzz*` functions as symbolic invariant tests when
   `--symbolic` is enabled.
4. Build symbolic calldata from the Alloy ABI type model.
5. Execute bytecode in a standalone symbolic EVM, reading concrete code and
   setup state from the Foundry executor backend.
6. Store mutations in a symbolic world overlay instead of mutating the concrete
   executor.
7. Query the solver for branch feasibility and model extraction.
8. Replay any candidate counterexample through the concrete Foundry executor
   before reporting it.

The symbolic EVM is intentionally separate from revm's concrete interpreter. It
uses Foundry and revm data structures for bytecode, accounts, environment, and
backend reads, but symbolic execution needs its own expression values, memory,
storage, call frames, path constraints, and solver integration.

Important internal pieces:

- `SymbolicExecutor` owns configuration and the solver backend.
- `SymbolicRunInput` describes one deployed test contract function to explore.
- `SymbolicInvariantRunInput` describes one bounded invariant sequence run.
- `SymbolicRunResult` and `SymbolicInvariantRunResult` return safe,
  counterexample, or incomplete outcomes.
- `SymbolicWorld` overlays balances, nonce, code, persistent storage, transient
  storage, logs, returndata, snapshots, and account lifecycle changes.
- `CallFrame` tracks address, code address, storage address, caller, call value,
  static context, calldata, stack, memory, and returndata.
- `SymbolicSolver` is the small internal trait used by the default SMT-LIB
  subprocess backend, which resolves named solvers (z3, cvc5, yices, bitwuzla,
  etc.) into solver-specific argv via `solver_commands_for_config`.

## EVM And World Semantics

The executor models byte-precise calldata, memory, returndata, logs, storage,
and symbolic storage keys. Supported behavior includes:

- arithmetic and bitwise operations, including symbolic-safe `EXP`,
  `SIGNEXTEND`, `BYTE`, shifts, and checked wrapping behavior
- `CALLDATALOAD`, `CALLDATACOPY`, `CODECOPY`, `EXTCODECOPY`,
  `RETURNDATACOPY`, `MCOPY`, `MLOAD`, `MSTORE`, and `MSTORE8`
- `CALL`, `STATICCALL`, `DELEGATECALL`, `CALLCODE`, `CREATE`, and `CREATE2`
- concrete and bounded symbolic call targets over known deployed contracts
- symbolic calldata and returndata across call frames
- `SLOAD`, `SSTORE`, `TLOAD`, and `TSTORE` with concrete or symbolic keys
- symbolic `KECCAK256` terms for Solidity mapping and dynamic-array storage
  patterns
- concrete code and storage read from the normal Foundry setup backend,
  including fork-backed setup state
- balances, value transfer, nonce lifecycle, code hash, account existence, and
  pre-Cancun `SELFDESTRUCT`
- common concrete-input precompile cases
- block and transaction environment reads and supported environment cheatcodes

Unsupported symbolic constructs return an incomplete result with a `Stuck`
reason instead of silently proving the property.

## Known limitations

Unsupported constructs report `Incomplete` rather than a proof. Some supported
semantics are bounded or approximate; in those cases, `PASS` only applies to the
modeled semantics and configured bounds.

Known incomplete, bounded, or approximate surfaces include:

| Area | Current behavior |
|---|---|
| Gas-dependent behavior | The engine does not use gas to prove properties. A raw `GAS` / `gasleft()` value is tolerated only as the direct gas operand to a CALL-family opcode and is not used to model gas availability. Explicit CALL-family gas caps are not enforced. Branches, arithmetic, call targets/values, calldata/returndata, memory/log offsets or sizes, `expectCall` gas matching, or solver constraints derived from observed gas report incomplete. Non-observable gas metering helpers are accepted as no-ops; observable gas read/snapshot helpers such as `lastCallGas`, `lastFrameGas`, `snapshotGasLastCall`, `snapshotGasLastFrame`, and `stopSnapshotGas` report incomplete and should not be used as symbolic properties. |
| `SELFDESTRUCT` | Pre-Cancun deletion is modeled. Cancun/EIP-6780 is modeled for concrete beneficiaries: contracts created in the current top-level symbolic transaction are deleted, while existing contracts transfer balance and halt without deleting code or storage. Unresolved symbolic Cancun beneficiaries report incomplete. |
| Symbolic account/code queries | `BALANCE`, `EXTCODESIZE`, `EXTCODEHASH`, and `EXTCODECOPY` on symbolic addresses are scoped to the engine's known symbolic/overlay/code-cache candidates plus the documented empty-account fallback. They do not prove quantified properties over every possible fork/backend account. |
| Symbolic CALL targets | Concrete targets and symbolic targets constrained to known deployed-contract/precompile candidates are supported. By default, a feasible symbolic target outside the known candidate set reports incomplete. With `symbolic_call_targets = true`, the outside-candidate branch is modeled as an empty-account/no-code successful call, including value transfer for `CALL`; it does not model arbitrary unknown external code or custom/future precompiles. Symbolic cheatcode addresses/selectors still report incomplete. |
| Symbolic CREATE / CREATE2 inputs | Concrete initcode and common bounded symbolic CREATE2 address expressions are supported. Symbolic runtime sizes and unsupported symbolic initcode shapes report incomplete. |
| ABI and calldata shape limits | Primitive ABI types, arrays, tuples, structs, bytes, and strings are supported within configured dynamic length and calldata byte limits. Unsupported ABI types, invalid ABI shapes, or calldata exceeding configured budgets report incomplete or config errors. |
| Dynamic memory and copy bounds | Many symbolic memory, calldata, returndata, and `MCOPY`/`RETURNDATACOPY` sizes are supported when bounded by configuration or solver-proved limits. Unbounded or out-of-bounds symbolic reads/copies report incomplete. |
| Concrete-required operands and bytecode | Symbolic data can flow through calldata, memory, storage, logs, and returndata, but some control/metadata values must resolve to concrete or solver-constrained values: `JUMP`/`JUMPI` destinations, `BLOBHASH` indices, cheatcode selectors, many cheatcode ABI decodes, fork IDs/block numbers, nonces, and created runtime bytecode opcodes. Symbolic bytecode opcodes, symbolic runtime sizes, or unconstrained control operands report incomplete. |
| Symbolic hashing and `KECCAK256` | Concrete hashes are computed exactly. Symbolic `KECCAK256` is represented by deterministic opaque terms plus Solidity-storage-layout heuristics for common mapping and dynamic-array keys. Proof obligations that depend on cryptographic facts such as non-zero hashes, collision resistance, or preimage resistance are not proof-grade and may report incomplete or produce replay-filtered candidates. |
| Symbolic storage base values | Writes and later reads through symbolic keys are modeled, with Solidity-layout heuristics for common mapping/dynamic-array keys. Reads of previously-unwritten symbolic keys are abstract storage variables by default, or zero under the zero-init storage layout; the engine does not enumerate arbitrary concrete backend storage slots for a symbolic key. Proofs involving unknown existing storage are scoped to the selected `symbolic.storage_layout`. |
| Precompiles | Canonical precompiles are recognized according to the active EVM version; KZG `0x0a` is Cancun+ only and falls back to normal empty-account behavior on earlier hardforks. Concrete inputs for modeled precompiles execute the corresponding revm precompile with effectively unlimited gas. Symbolic identity is byte-precise; symbolic hash/ecrecover/modexp outputs are deterministic opaque terms or fixed-length symbolic outputs, not full cryptographic/algebraic models. Symbolic BN254 inputs and symbolic BLAKE2f final flags report incomplete because precompile success depends on validity checks the symbolic model does not prove. KZG `0x0a` concrete inputs execute the revm KZG precompile exactly. Symbolic KZG calls model broad exact failures such as invalid input length and version/hash mismatches where known, plus selected replayable success/failure witnesses. Any remaining feasible symbolic KZG space reports incomplete rather than being treated as proved safe. Symbolic length headers, symbolic modexp output lengths, out-of-bounds symbolic inputs, future/custom precompiles, and precompile gas/OOG behavior are not fully modeled. |
| Hard arithmetic | Bit-vector arithmetic is modeled through SMT. Forge proves unsigned monotonic product and same-divisor quotient comparisons when path bounds show that every product fits in 256 bits. Other expensive arithmetic has bounded helpers, but unsupported `EXP` base/exponent shapes and solver-intractable nonlinear identities can report incomplete or timeout. |
| Cheatcode surface | The common testing cheatcodes listed below are modeled for safe concrete/symbolic forms. Unsupported Foundry/VM compatibility cheatcodes, value-bearing cheatcode calls, delegatecall prank forms, symbolic `expectCall` gas, unsupported symbolic `vm.bound` ranges, and unsupported symbolic `assumeNoRevert` decodes/overlaps report incomplete. |
| Approximate/no-op cheatcodes | Some recognized Foundry helpers are accepted but not semantically checked under symbolic execution, including non-observable gas metering helpers, access-list/warm/cool helpers, `allowCheatcodes`, `sleep`, and breakpoints. Observable EVM-version helpers, gas snapshot/read helpers, and safe-memory expectation helpers report incomplete instead of fabricating results or silently accepting assertions. |
| Fork mutation during symbolic execution | Fork-backed setup is allowed before symbolic execution. Creating forks, selecting a different fork, or rolling/mutating fork blocks during symbolic execution is restricted and reports incomplete unless it stays on the already active fork in the supported form. |
| Filesystem, JSON/TOML, and prompt-shaped inputs | Environment reads and `ffi` are supported for concrete values and commands, with `ffi` gated by Forge's existing `--ffi` setting. Missing or unparsable env values, disabled or failing FFI, non-UTF8 stdout, artifact lookup failures, filesystem access, JSON/TOML parsing or serialization, and interactive prompt cheatcodes report incomplete. |
| Resource and scope bounds | `max_paths` / width, execution depth, calldata variant budget, solver query budget, and solver timeout can stop a run as incomplete. Dynamic ABI length settings, `invariant_depth`, and `symbolic.loop` define the explored input/sequence/loop scope; a `PASS` is only within those configured bounds, and skipped larger shapes, deeper sequences, or more loop iterations are not necessarily reported as incomplete. |

Exact failure messages are preserved in the test output, for example
`unsupported symbolic execution feature: GAS/gasleft() not modeled`.

For real-world bug-shaped examples that exercise the current modeled surface,
see the community-maintained
[`symbolic-bug-suite`](https://github.com/grandizzy/symbolic-bug-suite). Those
examples are written so a successful symbolic run reports a concrete
counterexample.

## Cheatcodes

Symbolic tests run through a symbolic cheatcode handler for the subset that can
be modeled safely. The supported surface includes:

- `vm.assume`, `vm.bound`, `vm.skip`, `vm.assumeNoRevert`
- forge-std and DSTest assertion/failure signals
- `warp`, `roll`, `fee`, `chainId`, `difficulty`, `coinbase`, blockhash helpers
- `deal`, `store`, `load`, `etch`, `getCode`, `getDeployedCode`
- `prank`, `startPrank`, `stopPrank`
- `addr`, `sign`, `deriveKey`, `rememberKey`, `rememberKeys`
- `env*`, `envOr*`, `envExists`
- `ffi`, gated by Forge's existing `--ffi` setting
- console calls as no-ops
- logs and expectations such as `recordLogs`, `getRecordedLogs`, `expectEmit`
- call expectations and mocks for supported concrete and symbolic forms
- snapshots and state/storage helper calls used by symbolic tests

If a cheatcode is not modeled, the executor reports an incomplete symbolic run
with a clear unsupported-feature reason.

## Troubleshooting

### `No tests found` for a `check*` or `prove*` function

`check*` and `prove*` are symbolic entrypoints, not normal Forge tests. They are
discovered only when symbolic mode is enabled:

```sh
forge test --symbolic --match-test check_my_property
```

If Forge still prints `No tests found in project! Forge looks for functions that
start with test`, check the following:

1. The binary was built from a revision that includes native symbolic test
   discovery. `forge test --help` should list `--symbolic` options, and the
   source should include the `check*` / `prove*` symbolic entrypoint path.
2. The file is under the current project's configured test or source paths and
   is compiled by this `forge test` invocation. A file outside the project, or
   outside the active profile's `src` / `test` paths, can compile in another
   project but will not be discovered here.
3. `--match-test` filters function names/signatures. Use `--match-contract` for
   contract names:

   ```sh
   forge test --symbolic --match-contract MySymbolicTest
   ```

### `PASS` is surprising

First check whether the property depends on one of the known limitations above.
A `PASS` is scoped to the current symbolic model and configured bounds; it does
not cover skipped dynamic lengths, deeper invariant sequences, larger loop
bounds, unmodeled gas behavior, arbitrary unknown external code, or
cryptographic preimage/collision properties. If the property should have a
counterexample within the modeled surface, reduce the example and try raising
`symbolic.max_paths`, `symbolic.max_depth`, `symbolic.max_solver_queries`, or
the relevant dynamic length / invariant depth settings.

### `Incomplete` is not a proof

An incomplete run means the executor stopped before establishing safety or
replaying a counterexample. To continue, adjust bounds, simplify the property,
avoid the unsupported construct, or file a minimal repro for a missing model.

## Results

At the crate boundary, symbolic execution returns:

- `Safe`: all explored paths completed without a feasible failure.
- `Counterexample`: the solver found a model for a failing path. Forge must
  replay this before reporting it to the user.
- `Incomplete`: execution stopped before a proof or replayed counterexample.

Incomplete runs carry a stop reason:

- `Stuck`: unsupported symbolic construct or configured width/depth/query limit.
- `RevertAll`: every explored path ended in an ordinary revert.
- `Timeout`: solver timeout or solver `unknown`.
- `Error`: backend, ABI, bytecode, or solver process failure.

## Development Checks

Useful checks while changing this crate:

```sh
cargo fmt --check
cargo check -p foundry-evm-symbolic
cargo test -p foundry-evm-symbolic
cargo check -p forge
cargo test -p forge --test cli test_cmd::symbolic -- --nocapture
SYMBOLIC_CONFORMANCE=1 cargo test -p forge --test cli symbolic_conformance -- --nocapture
SYMBOLIC_LIMITS=1 cargo test -p forge --test cli symbolic_limits -- --nocapture
```

The conformance and limits suites are gated because they require Z3 and exercise
broader, slower symbolic behavior. The limits suite intentionally checks resource
boundaries such as path width, execution depth, calldata budgets, hard arithmetic,
and invariant sequence depth.
