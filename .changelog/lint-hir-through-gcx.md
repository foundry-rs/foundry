---
forge: patch
---

Internal cleanup of the Solidity linter: analyses read the HIR through the global context instead of carrying a separate handle; lint behavior is unchanged.
