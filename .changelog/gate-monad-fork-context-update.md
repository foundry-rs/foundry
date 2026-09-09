---
foundry-cheatcodes: patch
foundry-evm-core: patch
foundry-evm: patch
---

Kept the Monad fork-context update signal behind the `monad` feature so non-Monad builds no longer carry the `ContextUpdate` type and its construction sites.
