# `address` event parameters should be `indexed`

**Severity**: `Info`
**ID**: `event-fields`

Flags events whose `address` parameters are not declared `indexed`.

## What it does

Reports unindexed `address` and `address payable` event parameters when the event has no
indexed parameters. Contract, interface, and user-defined value types are excluded.

## Why restrict this?

Indexed event parameters are stored as topics in the transaction log, which lets
off-chain indexers, explorers, and clients efficiently filter events by sender or
recipient. Leaving filterable fields unindexed forces consumers to scan and decode every
event, which is slow and brittle.

Indexing has a gas cost and a limited topic budget. Leave a field unindexed when consumers do not
need to filter by it or when compatibility requires preserving the event's existing layout.

## Example

```solidity
event Transfer(address from, address to, uint256 value);
event Mint(address to, uint256 tokenId);
```

Use instead:

```solidity
event Transfer(address indexed from, address indexed to, uint256 value);
event Mint(address indexed to, uint256 tokenId);
```
