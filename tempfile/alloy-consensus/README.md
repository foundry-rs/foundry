# alloy-consensus

Ethereum consensus interface.

This crate contains constants, types, and functions for implementing Ethereum
EL consensus and communication. This includes headers, blocks, transactions,
[EIP-2718] envelopes, [EIP-2930], [EIP-4844], and more.

In general a type belongs in this crate if it is committed to in the EL block
header. This includes:

- transactions
- blocks
- headers
- receipts
- [EIP-2718] envelopes.

[alloy-network]: ../network
[EIP-2718]: https://eips.ethereum.org/EIPS/eip-2718
[EIP-2930]: https://eips.ethereum.org/EIPS/eip-2930
[EIP-4844]: https://eips.ethereum.org/EIPS/eip-4844

## Provenance

Much of this code was ported from [reth-primitives] as part of ongoing alloy
migrations.

[reth-primitives]: https://github.com/paradigmxyz/reth/tree/main/crates/primitives
