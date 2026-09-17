---
foundry-evm-symbolic: patch
---

Fixed symbolic tests and invariant runs wasting execution depth on ambiguous mapping SSTORE forks, which could spuriously report Incomplete when reaching the depth limit.
