---
cast: patch
---

`cast tx --raw` and `cast tx --lane` no longer panic on a transaction type Foundry does not model, such as the `ArbitrumInternalTx` (`0x6a`) that opens every Arbitrum and Orbit rollup block. They now report "Cannot EIP-2718 encode transaction type 0x6a" instead.
