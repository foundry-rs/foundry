---
foundry-cheatcodes: patch
foundry-evm-core: patch
foundry-evm: patch
---

Kept Monad transaction-state tracking and its fork-context transition handling behind the `monad` feature so non-Monad builds no longer carry Monad-only state.
