---
anvil: patch
---

Fixed a node panic when building the pending block if a Prague+ post-execution deposit-request log couldn't be decoded, instead of returning an RPC error.
