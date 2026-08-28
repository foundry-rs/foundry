---
anvil: patch
cast: patch
chisel: patch
forge: patch
foundry-common: patch
foundry-evm-core: patch
---

Treated `anvil_nodeInfo` as optional until an endpoint returns valid node info, while keeping later
Anvil identity checks strict. Zero-parameter `anvil_nodeInfo` and `anvil_metadata` calls now send
empty parameter arrays, allowing non-Anvil endpoints with vendor-specific rejection codes to be
forked.
