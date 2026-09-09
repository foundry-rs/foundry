# Unsafe OZ ERC721 mint

**Severity**: `Med`
**ID**: `unsafe-oz-erc721-mint`

Flags calls to OpenZeppelin's `ERC721._mint`, which credits a token without checking that the recipient can receive it.

## What it does

Reports calls to OpenZeppelin's ERC721 `_mint`, including overrides that delegate to it,
without a recognized recipient check.

Prefer `_safeMint` to check contract recipients. A custom check must either establish that
the recipient has no code or ask that recipient to accept the minted token after ownership
is established and revert on rejection. Custom checks may still produce warnings; review them before suppressing
the lint. Custom mint implementations that do not use OpenZeppelin's `_mint` are outside
this rule's scope.

## Why is this bad?

`ERC721._mint` assigns the token without calling `onERC721Received` on the recipient. A recipient
contract without a way to transfer the token onward may leave it permanently inaccessible, though
lack of the receiver interface alone does not prove that it is locked. `_safeMint` checks receiver
acceptance and reverts on rejection. Its receiver callback is an external interaction, so arrange
state changes and reentrancy protections accordingly.

## Example

```solidity
function mint(address to, uint256 id) external {
    _mint(to, id);
}
```

Use instead:

```solidity
function mint(address to, uint256 id) external {
    _safeMint(to, id);
}
```
