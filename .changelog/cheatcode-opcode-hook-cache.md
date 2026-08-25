---
forge: patch
foundry-evm: patch
foundry-cheatcodes: patch
---

Sped up `forge test` further by caching the cheatcode per-opcode hook predicates instead of re-testing them for every executed opcode.
