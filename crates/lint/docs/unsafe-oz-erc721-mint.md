# Unsafe OZ ERC721 mint

**Severity**: `Med`
**ID**: `unsafe-oz-erc721-mint`

Flags calls that resolve to `ERC721._mint`, which credits a token without checking that the recipient can receive it.

## What it does

Reports a call whose callee resolves to a function named `_mint` declared in a contract named exactly `ERC721`, `ERC721Upgradeable`, `ERC721Consecutive` or `ERC721ConsecutiveUpgradeable` whose source comes from an OpenZeppelin package path, wherever that contract sits in the caller's inheritance chain, or to a user `_mint` override that transitively delegates to one of those (the capped/pausable pattern forwarding through `super._mint`): direct calls dispatch to the override, but the path still reaches the unchecked base. The first two canonical contracts declare the unchecked `_mint`, and most extensions (`ERC721Enumerable`, ...) inherit it, so resolution lands on the base; the v4 Consecutive extensions are the exception, they override `_mint` with a construction guard that forwards to the base through `super._mint`, still without a receiver check (in v5 they override `_update` instead, and their names match nothing). The plain `_mint(to, id)`, the qualified `ERC721._mint(to, id)` and `super._mint(to, id)` forms are all covered. Exact names keep a safe override clean even when the overriding contract's name contains the `ERC721` substring, and the provenance requirement keeps a local contract reusing a canonical name out of scope; a vendored OpenZeppelin copy under a path that does not name OpenZeppelin is not recognized.

This mirrors Aderyn's `unsafe-oz-erc721-mint` detector, with a resolution-based check instead of its name-and-import heuristic: Aderyn flags any identifier named `_mint` in a file that imports an `openzeppelin` path and whose contract lists a direct `ERC721*` base, which misses indirect inheritance and depends on the import path. Resolving the callee also keeps `ERC20._mint` and unrelated local `_mint` functions out of scope.

Two exemptions:

- calls to `_safeMint` are the recommended fix and never fire;
- calls made inside the canonical wrapper, a `_safeMint` declared in `ERC721`/`ERC721Upgradeable` itself, which legitimately calls `_mint` next to its receiver check. A user-defined `_safeMint` override stays analyzed: it can call `_mint` directly without any check;
- calls made inside a user `_mint` override: the override is the mint primitive itself, `super._mint` there is delegation, and `_safeMint` there would re-enter the override through the virtual dispatch. A delegating override reports at its call sites instead, unless it performs the receiver check on the minted recipient before forwarding, a resolved `onERC721Received` call or a `.code` inspection mentioning the recipient parameter: such a wrapper is safe like the canonical `_safeMint`, and its callers stay clean. A check on an unrelated address, `address(this).code` construction guards included, does not count.

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
