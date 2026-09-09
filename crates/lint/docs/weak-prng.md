# Weak pseudo-random number generation

**Severity**: `Med`
**ID**: `weak-prng`

## What it does

Reports direct use of `block.timestamp`, `block.number`, `block.coinbase`, `blockhash(...)`,
`block.prevrandao`, or `block.difficulty` in modulo expressions or `keccak256(...)`. `abi.encode*`
calls are treated as entropy only when they feed one of those expressions.

`block.difficulty` is included for legacy code. On proof-of-stake chains it is equivalent to
`block.prevrandao` and has been deprecated since Solidity 0.8.18.

## Why is this bad?

Block data is visible before transaction execution and can often be influenced or withheld by a
block proposer. Hashing or applying modulo to these values does not make them unpredictable, so an
attacker may be able to bias outcomes such as lotteries, mints, or game mechanics.

Use a commit-reveal scheme, an oracle such as a VRF, or another protocol designed for
unpredictable randomness.

## Example

```solidity
uint256 winner = uint256(keccak256(abi.encodePacked(block.timestamp, msg.sender))) % players.length;
```

Use instead:

```solidity
// Example shape only: consume randomness that was committed before it was revealed.
uint256 winner = uint256(keccak256(abi.encodePacked(revealedSeed, msg.sender))) % players.length;
```

## Notes

Values copied into locals and entropy obtained through helpers or inline assembly may go
unreported. The absence of a warning is not evidence that a randomness source is unpredictable.

Time-bucketing expressions such as `block.timestamp % 1 days` are excluded when the bucket
is a constant whole-day multiple. Shorter or variable buckets may still warn; review
their intended use before suppressing the lint.
