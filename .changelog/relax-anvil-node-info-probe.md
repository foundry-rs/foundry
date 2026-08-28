---
anvil: patch
cast: patch
forge: patch
foundry-common: patch
foundry-evm-core: patch
---

Treated failed `anvil_nodeInfo` calls as an unavailable optional capability until an endpoint is
identified as Anvil, kept later validation strict across resets, and sent valid empty parameter
arrays for Anvil metadata requests.
