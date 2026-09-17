---
forge: patch
foundry-evm-symbolic: patch
---

Improve symbolic reasoning about fixed-point round trips, including zero balances and full-width balances and rates constrained by successful overflow checks instead of explicit upper bounds. Infer operand bounds from non-overflowing products, and simplify signed arithmetic guards and contradictory scalar bounds while preserving overflow and rounding counterexamples.
