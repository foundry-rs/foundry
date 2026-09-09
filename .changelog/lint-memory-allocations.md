---
forge: patch
forge-lint: patch
---

Stop treating memory allocation as an external call in calls-loop and reentrancy-events, while preserving diagnostics for contract creation and external allocation-length calculations.
