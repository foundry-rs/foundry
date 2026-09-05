---
forge-lint: patch
---

`reentrancy-balance` now detects the "delta form" of a stale-balance comparison guard (`balanceAfter - balanceBefore >= amount`), not just the direct-addition form (`balanceAfter >= balanceBefore + amount`). Both are algebraically identical and equally vulnerable to reentrancy, but only the direct form was previously flagged.
