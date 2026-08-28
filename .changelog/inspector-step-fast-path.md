---
forge: patch
foundry-evm: patch
foundry-cheatcodes: patch
---

Sped up `forge test` by caching per-opcode inspector hooks and keeping non-storage opcodes on the fast path while `vm.record` is active.
