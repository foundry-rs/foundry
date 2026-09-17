---
forge: patch
foundry-evm-symbolic: patch
---

Fixed symbolic invariant checks to ignore boolean return values, matching concrete Forge behavior. Invariants must use assertions or revert to indicate failure.
