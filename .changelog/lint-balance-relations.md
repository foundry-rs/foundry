---
forge-lint: patch
---

Recognize stale-balance differences through arithmetic offsets, locals, and internal helper returns in `reentrancy-balance`. Preserve direct comparisons with conditional operands and transformed balances, including through assignments and helper returns, without treating unsupported same-side arithmetic as a balance difference.
