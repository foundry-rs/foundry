---
forge-lint: patch
---

Recognize stale-balance differences through arithmetic offsets, locals, and internal helper returns in `reentrancy-balance`, preserve direct comparisons with transformed balances and conditional operands, and avoid treating unsupported same-side arithmetic as a balance difference.
