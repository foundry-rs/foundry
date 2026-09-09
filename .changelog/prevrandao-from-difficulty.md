---
foundry-evm-core: patch
cast: patch
forge: patch
anvil: patch
---

Exposed `difficulty` as `block.prevrandao` when forking Polygon, Avalanche and Arbitrum, matching what those chains return on-chain.
