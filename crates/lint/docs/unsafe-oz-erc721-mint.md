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
- calls made inside a user `_mint` override: the override is the mint primitive itself, `super._mint` there is delegation, and `_safeMint` there would re-enter the override through the virtual dispatch. A delegating override reports at its call sites instead, unless the delegated mint reverts when the recipient refuses the token: such a wrapper is safe like the canonical `_safeMint`, and its callers stay clean.

The recognized guards are a closed set, because a hook call that merely appears somewhere in a condition proves nothing about whether the revert depends on its answer. An override reverts on refusal when it asks `onERC721Received` of the recipient itself and:

- `require` or `assert` holds the comparison `hook(to) == selector` as its whole condition, or the account short circuit `to.code.length == 0 || hook(to) == selector`;
- or `if (hook(to) != selector)` takes a branch that always reverts, or `if (hook(to) == selector)` has an `else` that does. A branch reverts only when nothing before its `revert` can leave the function keeping the token: a `return`, an assembly block, a `try` or a `switch` all end the reading there;
- or one of those sits under a test that the recipient carries code (`> 0`, `!= 0`, `>= 1`, or their mirrors), an account needing no hook;
- or one of those is reached through a function or a modifier the recipient itself is handed to, the way OpenZeppelin factors `_checkOnERC721Received` out of `_safeMint`. A `virtual` callee does not count: an override may replace its body, and the one analyzed here is the statically resolved declaration.

The guard may follow the delegation, since the revert undoes the mint, and it covers the recipient it names, read from the argument bound to the callee's first parameter, named arguments included. An override handing the base any other address reports. The hook is matched on its `(address, address, uint256, bytes)` shape. The answer it is compared against must be `0x150b7a02`, spelled, converted, named by an `onERC721Received.selector` member, or held by a `constant`. An `immutable` or a state variable does not exempt, its value being unknown here.

Everything else reports, including an override that only inspects the recipient's `.code` or restricts the mint to code-less recipients, a hook whose answer is discarded, stored in a local, returned as a `bool` by a helper, or wrapped in a `try` whose `catch` may swallow the refusal, a hook a second operand can short circuit past, a hook riding in the revert message, one asked of an address derived from the recipient rather than of the recipient, one guarded inside a loop body that may never run, one whose refusal only `return`s, one whose guard lives in a `virtual` callee, and an exiting branch taken on acceptance instead of refusal. Following an answer across statements would take a dataflow analysis this detector does not run, which also reports a wrapper whose mint sits in the accepting branch and whose refusal falls through to a sibling `revert`.

One shape stays out of reach: an override that reimplements the mint itself, assigning ownership without delegating to the OpenZeppelin base, never resolves to the unchecked `_mint` and is not reported, even though it locks a token just the same.

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
