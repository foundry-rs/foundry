# Encode Packed Collision

**Severity**: `High`
**ID**: `encode-packed-collision`

`abi.encodePacked()` with multiple dynamic-type arguments produces ambiguous encodings. Because packed encoding concatenates values without length prefixes, different inputs can produce the same output — for example, `encodePacked("a", "bc") == encodePacked("ab", "c")`. When the result is hashed and used as a key or signature, this enables collision attacks.

## What it does

Flags calls to `abi.encodePacked()` where two or more arguments have dynamic types:

- `string`
- `bytes` (dynamic)
- dynamic arrays (`T[]`)

## Why is this bad?

Hash collisions allow an attacker to craft inputs that hash to an identifier they do not own. Common vulnerable patterns include:

- Merkle leaf construction: an attacker submits two adjacent leaves concatenated to match a sibling pair
- Signature payloads: two different messages that produce the same signature hash
- Access-control keys: two different (user, resource) pairs that map to the same key

This lint is intentionally conservative and flags by argument type, not by proving exploitability.
Some injective patterns, such as repeating the same dynamic value or manually adding length prefixes,
may still be reported.

## Example

### Bad

```solidity
function getKey(string memory a, string memory b) public pure returns (bytes32) {
    return keccak256(abi.encodePacked(a, b)); // "a"+"bc" == "ab"+"c"
}
```

### Good

Use `abi.encode()` instead — it includes length prefixes that prevent collisions:

```solidity
function getKey(string memory a, string memory b) public pure returns (bytes32) {
    return keccak256(abi.encode(a, b));
}
```
