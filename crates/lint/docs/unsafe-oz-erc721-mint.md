# Unsafe OZ ERC721 mint

**Severity**: `Med`
**ID**: `unsafe-oz-erc721-mint`

Flags calls that resolve to `ERC721._mint`, which credits a token without checking that the recipient can receive it.

## What it does

Reports a call whose callee resolves to a function named `_mint` declared in a contract whose name contains `ERC721` (`ERC721`, `ERC721Upgradeable`, `ERC721Enumerable`, ...), wherever that contract sits in the caller's inheritance chain. The plain `_mint(to, id)`, the qualified `ERC721._mint(to, id)` and `super._mint(to, id)` forms are all covered.

This mirrors Aderyn's `unsafe-oz-erc721-mint` detector, with a resolution-based check instead of its name-and-import heuristic: Aderyn flags any identifier named `_mint` in a file that imports an `openzeppelin` path and whose contract lists a direct `ERC721*` base, which misses indirect inheritance and depends on the import path. Resolving the callee also keeps `ERC20._mint` and unrelated local `_mint` functions out of scope.

Two exemptions:

- calls to `_safeMint` are the recommended fix and never fire;
- calls made inside a function named `_safeMint` are the wrapper implementation itself, which legitimately calls `_mint` next to its receiver check.

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
