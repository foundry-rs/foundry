---
forge: patch
foundry-evm-symbolic: patch
---

Improve symbolic reasoning about fixed-point round trips, including zero balances and rates constrained by successful overflow checks instead of an explicit upper bound. Simplify signed arithmetic guards and contradictory scalar bounds while preserving overflow and rounding counterexamples.
