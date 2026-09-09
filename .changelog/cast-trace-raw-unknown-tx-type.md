---
cast: patch
---

`cast trace --raw` no longer panics when a transaction passed as JSON has a type Foundry does not model, such as the `ArbitrumInternalTx` (`0x6a`) that opens every Arbitrum and Orbit rollup block. It now reports "Cannot EIP-2718 encode transaction type 0x6a" instead.
