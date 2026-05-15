# Address and id event parameters should be indexed

**Severity**: `Info`
**ID**: `event-fields`

Flags events whose `address` or id-like (`uint256`/`bytes32` named like `id`, `*Id`,
`*ID`, `*_id`, `*_ID`) parameters are not declared `indexed`.

## What it does

For each event, identifies unindexed `address`, `address payable`, `uint256`, and
`bytes32` parameters that look filterable (addresses or id-like names) and reports a
single warning listing them. The lint respects the EVM cap on indexed parameters
(3 for normal events, 4 for `anonymous`) and does not flag events that are already at
capacity. Events that already have at least one indexed parameter are left alone:
the author has clearly chosen what to index, so we stay silent.

## Why is this bad?

Indexed event parameters are stored as topics in the transaction log, which lets
off-chain indexers, explorers, and clients efficiently filter events by sender,
recipient, token id, order id, etc. Leaving filterable fields unindexed forces
consumers to scan and decode every event, which is slow and brittle.

## Example

### Bad

```solidity
event Transfer(address from, address to, uint256 value);
event Mint(address to, uint256 tokenId);
event Order(bytes32 orderId, uint256 amount);
```

### Good

```solidity
event Transfer(address indexed from, address indexed to, uint256 value);
event Mint(address indexed to, uint256 indexed tokenId);
event Order(bytes32 indexed orderId, uint256 amount);
```

## Limitations

This lint is intentionally conservative:

- Only explicit `address`, `address payable`, `uint256`, and `bytes32` parameters
  are checked. Custom types (contract types, interfaces, user-defined value types)
  are **not** unwrapped, so e.g. `IERC20 token` or `type UserId is uint256;`
  parameters are not flagged.
- For `uint256`/`bytes32`, only names matching `id`, `*Id`, `*ID`, `*_id`, or
  `*_ID` are flagged. Other names (e.g. `amount`, `hash`, `nonce`) are ignored.
- Only actionable suggestions are reported. If an event has no remaining indexed
  slots (3 for normal events, 4 for `anonymous`), no warning is emitted. If only
  some slots remain, only the first parameters that could still be indexed are
  reported; already-indexed parameters are never suggested for change.
