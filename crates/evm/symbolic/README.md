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
- `check*` and `prove*` tests are only selected when `--symbolic` is enabled.
- A reported counterexample must replay concretely before Forge prints it as a
  failure.

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
# solver_portfolio = ["z3", "cvc5", "bitwuzla"]
timeout = 30
max_depth = 10000
max_paths = 1024
max_solver_queries = 10000
default_dynamic_length = 2
max_dynamic_length = 256
array_lengths = []
dynamic_lengths = {}
default_array_lengths = []
default_bytes_lengths = []
max_calldata_bytes = 4096
invariant_depth = 10
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
forge test --symbolic --symbolic-solver-portfolio z3,cvc5,bitwuzla
forge test --symbolic --symbolic-timeout 120
forge test --symbolic --symbolic-array-lengths 2,4
forge test --symbolic --symbolic-invariant-depth 6
forge test --symbolic --symbolic-call-targets
forge test --symbolic --symbolic-dump-smt

FOUNDRY_SYMBOLIC=true forge test
FOUNDRY_SYMBOLIC_SOLVER=z3 forge test --symbolic
FOUNDRY_SYMBOLIC_SOLVER_COMMAND="z3 -in -smt2" forge test --symbolic
FOUNDRY_SYMBOLIC_SOLVER_PORTFOLIO="z3,cvc5,bitwuzla" forge test --symbolic
FOUNDRY_SYMBOLIC_TIMEOUT=120 forge test --symbolic
```

Known solver names are `z3`, `yices`, `cvc5`, `cvc5-int`, `bitwuzla`, and
`bitwuzla-abs`. Unknown `symbolic.solver` values are treated as z3-compatible
executables and are invoked with `-in -smt2` to preserve the old
`symbolic.solver = "/path/to/z3"` behavior. Use `symbolic.solver_command` for
non-z3-compatible command lines or wrapper tools.

`symbolic.solver_portfolio` runs multiple solvers in parallel for each SMT query
and uses the first `sat` or `unsat` response. `unknown` results only win if no
configured solver returns a decisive response. A nonempty `symbolic.solver_command`
overrides both `symbolic.solver_portfolio` and `symbolic.solver`; otherwise a
nonempty portfolio overrides `symbolic.solver`. Portfolio entries without
whitespace, quotes, or backslashes are resolved like `symbolic.solver` values.
Entries with whitespace, quotes, or backslashes are split into argv parts like
`symbolic.solver_command`; they are not executed through a shell.

Security note: `symbolic.solver_command` and `symbolic.solver_portfolio` execute
local programs when symbolic tests run. This also applies when these values come
from inline `forge-config:` or translated legacy `@custom:halmos` annotations.
Review solver settings before running symbolic tests from untrusted projects.

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
- `SymbolicSolver` is the small internal trait used by the default Z3 subprocess
  backend.

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
- balances, value transfer, nonce lifecycle, code hash, account existence, and
  `SELFDESTRUCT`
- common concrete-input precompile cases
- block and transaction environment reads and supported environment cheatcodes

Unsupported symbolic constructs return an incomplete result with a `Stuck`
reason instead of silently proving the property.

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
