# Weak pseudo-random number generation

**Severity**: `Med`
**ID**: `weak-prng`

Flags randomness-like expressions that directly derive entropy from predictable on-chain values.

## What it does

Reports direct use of `block.timestamp`, `block.number`, `block.coinbase`, `blockhash(...)`,
`block.prevrandao`, or `block.difficulty` in modulo expressions, `keccak256(...)`, or
`abi.encode*` calls.

`block.difficulty` is included for legacy code. On proof-of-stake chains it is equivalent to
`block.prevrandao` and has been deprecated since Solidity 0.8.18.

## Why is this bad?

Block data is visible before transaction execution and can often be influenced or withheld by a
block proposer. Hashing or applying modulo to these values does not make them unpredictable, so an
attacker may be able to bias outcomes such as lotteries, mints, or game mechanics.

Use a commit-reveal scheme, an oracle such as a VRF, or another protocol designed for
unpredictable randomness.

## Example

### Bad

```solidity
uint256 winner = uint256(keccak256(abi.encodePacked(block.timestamp, msg.sender))) % players.length;
```

### Good

```solidity
// Example shape only: consume randomness that was committed before it was revealed.
uint256 winner = uint256(keccak256(abi.encodePacked(revealedSeed, msg.sender))) % players.length;
```

## Notes

This lint is intentionally local and conservative. It does not attempt interprocedural taint
tracking, so values copied into a variable before use may require manual review.

The lint ignores obvious time-bucketing expressions such as `block.timestamp % 1 days`,
`block.timestamp % 86400`, and `block.timestamp % (24 * 60 * 60)`. Variable moduli such as
`block.timestamp % period` are still reported because the lint cannot infer whether the variable is
a duration or a randomness upper bound.

Only Solidity expressions are inspected. Inline assembly/Yul entropy sources such as `timestamp()`
or `number()` are out of scope.
