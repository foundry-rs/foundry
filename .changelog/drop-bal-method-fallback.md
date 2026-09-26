---
anvil: patch
forge: patch
---

Forked block access lists are now fetched only through the specified `eth_getBlockAccessList` method, without retrying the non-standard `eth_getBlockAccessListByBlockHash`.
