---
forge: patch
foundry-evm: patch
foundry-cheatcodes: patch
---

Sped up `forge test` in suites that use `stdstore` or `deal` by keeping non-storage opcodes on the fast path while `vm.record` is active.
