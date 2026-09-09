# Environment reads across Foundry mutations

**Severity**: `Med`
**ID**: `environment-read-across-mutation`

## What it does

Warns when a raw environment value can be used after a Foundry cheatcode changes that
environment, or when matching raw reads occur on both sides of a mutation in the same
call frame. The diagnostic points to the original read and recommends a getter when one exists.

This rule replaces `block-number-across-roll` and `block-timestamp-across-warp`. Update
`--only-lint`, `exclude_lints`, and inline suppressions to the new ID.

## Why is this bad?

Solidity compilers can reuse, move, or defer environment reads that are invariant during
normal EVM execution. Foundry can change these values within a test. Assigning a raw read
to a local does not guarantee that the compiler captures its value before a cheatcode call.
This affects optimized via-IR compilation as well as other compiler optimizations.

The rule recognizes the following reads and direct setters by their resolved ABI signature
and the constant cheatcode address, even when the receiver is an alias or helper argument:

| Read | Direct setter | Materialized getter |
| --- | --- | --- |
| `block.number` | `roll(uint256)` | `vm.getBlockNumber()` |
| `block.timestamp` | `warp(uint256)` | `vm.getBlockTimestamp()` |
| `block.chainid` | `chainId(uint256)` | `vm.getChainId()` |
| `block.coinbase` | `coinbase(address)` | External helper |
| `block.difficulty`, `block.prevrandao` | `difficulty(uint256)`, `prevrandao(bytes32)`, `prevrandao(uint256)` | External helper |
| `block.basefee` | `fee(uint256)` | External helper |
| `block.blobbasefee` | `blobBaseFee(uint256)` | `vm.getBlobBaseFee()` |
| `tx.gasprice` | `txGasPrice(uint256)` | External helper |
| `blockhash(n)` | `setBlockhash(uint256,bytes32)`, `roll(uint256)` | External helper |
| `blobhash(i)` | `blobhashes(bytes32[])` | `vm.getBlobhashes()[i]` |
| `block.gaslimit`, `block.slotnum` (Amsterdam) | Fork changes and snapshot restoration | External helper |

`roll` also affects `blockhash` because its valid history window depends on the current
block number. Both difficulty names read the same opcode, whose meaning depends on the
EVM version. `blockhash` and `blobhash` results retain dependencies on their index arguments.

All overloads of `selectFork`, `createSelectFork`, and `rollFork` are recognized. Fork
changes replace block/configuration fields and blockhash history; switching forks also
switches fork-scoped gas-price and blob-hash overrides. `revertToState` and
`revertToStateAndDelete`, plus their deprecated `revertTo` and `revertToAndDelete` aliases,
can restore all listed environments. These operations are treated conservatively: the rule
does not prove that a snapshot exists, a fork differs, or an explicitly rolled fork is active.
Creating a fork without selecting it is not a mutation of the current environment.

`prank` and `broadcast` affect subsequent call frames; they do not change the current frame's
`msg.sender`, `msg.value`, or `tx.origin` across the setter call. Balance, code, storage,
return-data, and gas reads are not in this invariant-environment class: the compiler already
accounts for their changes across calls. Inline assembly is outside this rule's analysis.

## Example

### Bad

```solidity
uint256 saved = block.chainid;
vm.chainId(2);
vm.chainId(saved); // The compiler need not have captured the original chain ID.
```

### Good

```solidity
uint256 saved = vm.getChainId();
vm.chainId(2);
vm.chainId(saved);
```

For fields without a getter, use a public/external helper and call it externally:

```solidity
function baseFee() external view returns (uint256) {
    return block.basefee;
}

function example() public {
    uint256 saved = this.baseFee();
    vm.fee(2);
    vm.fee(saved);
}
```

An internal helper can be inlined and does not provide this guarantee. Use getters or
external helpers on both sides of before/after comparisons. Keep compiler optimizations enabled.
When replacing `blobhash(i)` with array indexing, handle out-of-range indices if needed:
the opcode returns zero, whereas indexing the getter's returned array reverts.

## Scope and controls

The rule runs in `forge lint` and the normal build lint stage, including configured test
and script directories. It does not change compiler output or automatically insert cheatcodes.
Reads without a recognized matching mutation remain quiet.

The analysis follows scalar locals, arithmetic, tuples, internal helper arguments and
returns, inherited helpers, and modifiers. External call results, including public and
external library calls, are materialized values from separate call frames.

This is a bounded warning, not a complete execution analysis: it uses a 16,384-step budget,
retains at most 32 paths at statement boundaries, follows at most eight function frames, and
visits at most two loop iterations. It does not prove relationships between runtime conditions
or analyze recursive/indirect calls, low-level cheatcode calls, assembly, or heap/storage aliases.
Known unsigned and boolean locals prune exhausted loops and constant branches. Differing
helper return values and conditional-expression values are discarded rather than combining
mutually exclusive outcomes, which can miss captures returned by branching helpers. Hash
indices and fork/snapshot identities are not tracked precisely. Absence of a warning does not
establish that every test capture is safe.

Severity filters, `exclude_lints`, and inline suppressions apply. Suppress at the raw capture:

```solidity
// forge-lint: disable-next-line(environment-read-across-mutation)
uint256 saved = block.chainid;
```

This is separate from `block-timestamp`, which warns about validator-influenced comparisons
in production code.
