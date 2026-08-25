---
forge: patch
foundry-evm: patch
foundry-cheatcodes: patch
---

Sped up `forge test` by giving the per-opcode inspector hooks a small fast path that inlines into the interpreter loop.
