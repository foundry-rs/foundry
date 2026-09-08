---
foundry-cli: patch
forge: patch
---

Avoid repeated transitive import walks when preparing Solidity sources, reducing build preparation work for deep dependency graphs.
