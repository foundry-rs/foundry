# Encode Packed Collision

**Severity**: `High`
**ID**: `encode-packed-collision`

## What it does

Flags calls to `abi.encodePacked()` where two or more non-literal arguments have dynamic types:

- `string`
- `bytes` (dynamic)
- dynamic arrays (`T[]`)

String, hexadecimal-string, and Unicode-string literals do not count toward that total.

## Why is this bad?

Packed encoding omits dynamic-value boundaries, so different inputs can produce identical
encoded bytes and therefore the same hash. This is an ambiguous encoding, not a weakness in
the hash function. It can make distinct signature payloads or access-control keys indistinguishable.

Unambiguous encodings that repeat the same dynamic value or add length prefixes may still
be reported.

## Example

```solidity
function getKey(string memory a, string memory b) public pure returns (bytes32) {
    return keccak256(abi.encodePacked(a, b)); // "a"+"bc" == "ab"+"c"
}
```

Use instead:

Use `abi.encode()` instead — it includes length prefixes that prevent collisions:

```solidity
function getKey(string memory a, string memory b) public pure returns (bytes32) {
    return keccak256(abi.encode(a, b));
}
```
