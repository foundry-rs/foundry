# Unsafe OZ ERC721 mint

**Severity**: `Med`
**ID**: `unsafe-oz-erc721-mint`

Flags calls that resolve to `ERC721._mint`, which credits a token without checking that the recipient can receive it.

## What it does

Reports a call whose callee resolves to a function named `_mint` declared in a contract named exactly `ERC721` or `ERC721Upgradeable` (the OZ contracts that declare the unchecked `_mint`; extensions such as `ERC721Enumerable` inherit it, so resolution still lands on the base), wherever that base sits in the caller's inheritance chain. The plain `_mint(to, id)`, the qualified `ERC721._mint(to, id)` and `super._mint(to, id)` forms are all covered, and exact names keep a safe override clean even when the overriding contract's name contains the `ERC721` substring.

This mirrors Aderyn's `unsafe-oz-erc721-mint` detector, with a resolution-based check instead of its name-and-import heuristic: Aderyn flags any identifier named `_mint` in a file that imports an `openzeppelin` path and whose contract lists a direct `ERC721*` base, which misses indirect inheritance and depends on the import path. Resolving the callee also keeps `ERC20._mint` and unrelated local `_mint` functions out of scope.

Two exemptions:

- calls to `_safeMint` are the recommended fix and never fire;
- calls made inside the canonical wrapper, a `_safeMint` declared in `ERC721`/`ERC721Upgradeable` itself, which legitimately calls `_mint` next to its receiver check. A user-defined `_safeMint` override stays analyzed: it can call `_mint` directly without any check.

## Why is this bad?

`ERC721._mint` assigns the token without calling `onERC721Received` on the recipient. Minting to a contract that does not implement the receiver interface permanently locks the token. `_safeMint` performs the check and reverts instead.

## Example

### Bad

```solidity
function mint(address to, uint256 id) external {
    _mint(to, id);
}
```

### Good

```solidity
function mint(address to, uint256 id) external {
    _safeMint(to, id);
}
```
