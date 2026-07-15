# Unsafe ecrecover

**Severity**: `Med`
**ID**: `ecrecover`

Flags direct calls to Solidity's `ecrecover` builtin when the signature's `s` value is not proven
to be in the canonical lower half of the secp256k1 curve order.

## What it does

The lint follows simple local aliases and recognizes low-`s` bounds established by `require`,
`assert`, conditionals, and early-return or revert branches. It accepts the canonical maximum
`0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0`, stricter bounds, reversed
comparisons, and the equivalent strict comparison against that value plus one. Facts are
invalidated by reassignment, loop-carried writes, and calls that may change mutable state. They are
retained only when they hold on every path reaching the call.

Checks on `v`, the recovered address, nonces, domain separators, or the top bit of an EIP-2098
signature do not prove that `s` is canonical and do not suppress the warning.

The analysis is intentionally local and requires the low-`s` guard to be established before the
call. It does not summarize internal helpers or modifiers, prove post-call guards, recognize bound
signature schemes that fix one recovery ID, or inspect Yul and low-level calls to precompile
address `0x01`.

## Why is this bad?

For each valid high-`s` ECDSA signature, an attacker can derive a second signature for the same
message and signer by replacing `s` with its complement in the curve order and flipping `v`.
Contracts that use the signature bytes as a unique identifier can therefore have replay or
double-use protections bypassed. The `ecrecover` precompile does not enforce the low-`s` rule that
Ethereum transactions enforce.

## Example

### Bad

```solidity
function recover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) pure returns (address) {
    return ecrecover(hash, v, r, s);
}
```

### Good

```solidity
uint256 constant HALF_ORDER =
    0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

function recover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) pure returns (address) {
    require(uint256(s) <= HALF_ORDER, "invalid signature s");
    return ecrecover(hash, v, r, s);
}
```
