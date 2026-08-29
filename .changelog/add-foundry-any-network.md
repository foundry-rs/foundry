---
foundry-evm-core: minor
foundry-common: minor
---

Added `FoundryAnyNetwork`, a catch-all network with a Foundry-owned `FoundryAnyTxEnvelope` wrapper that supports RLP decoding and signer recovery, and wired it up as `AnyEvmNetwork` so the EVM abstraction can run against networks with unknown transaction types.
