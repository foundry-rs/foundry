---
foundry-evm-core: patch
foundry-evm: patch
cast: patch
anvil: patch
forge-verify: patch
---

Derived the execution hardfork from the fork block header for chains without a known activation schedule, so `cast run`, `cast call --trace` and `anvil --fork-url` no longer run ahead of the forked chain.
