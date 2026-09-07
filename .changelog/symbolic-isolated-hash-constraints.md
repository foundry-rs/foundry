---
forge: patch
foundry-evm-symbolic: patch
---

Sped up symbolic tests by locally solving independent constraints over opaque hash results instead of sending them to the SMT solver.
