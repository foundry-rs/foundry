---
anvil: patch
foundry-primitives: patch
---

Serve receipts for transaction types Foundry does not model, such as Arbitrum's, when forking instead of failing with "Failed to decode receipt". `debug_getRawTransaction` and `debug_getRawTransactions` no longer panic on those transactions, and fork transaction replay skips the prefix transactions it cannot execute rather than aborting.
