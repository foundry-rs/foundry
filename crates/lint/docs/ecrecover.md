# Unsafe `ecrecover`

**Severity**: `Med`
**ID**: `ecrecover`

## What it does

Reports direct `ecrecover` calls without a check that `s` is at most
`0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0` before the recovered
address is used.

Checking `v`, nonces, or the recovered address does not replace this low-`s` check.
Use a signature-validation library that enforces it. A custom validation helper may
still produce a warning; review its guarantees before suppressing the lint.

## Why is this bad?

For each valid high-`s` ECDSA signature, an attacker can derive a second signature for the same
message and signer by replacing `s` with its complement in the curve order and flipping `v`.
Contracts that use the signature bytes as a unique identifier can therefore have replay or
double-use protections bypassed. The `ecrecover` precompile does not enforce the low-`s` rule that
Ethereum transactions enforce.

## Example

```solidity
function recover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) pure returns (address) {
    return ecrecover(hash, v, r, s);
}
```

Use instead:

```solidity
uint256 constant HALF_ORDER =
    0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

function recover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) pure returns (address) {
    require(uint256(s) <= HALF_ORDER, "invalid signature s");
    return ecrecover(hash, v, r, s);
}
```
