---
anvil: patch
cast: patch
chisel: patch
forge: patch
foundry-common: patch
foundry-evm-core: patch
---

Treated `anvil_nodeInfo` as optional until an endpoint returns valid node info, while keeping later
Anvil identity checks strict across discovery and resets. This allows non-Anvil fork endpoints with
vendor-specific rejection codes to proceed through standard RPC discovery.
