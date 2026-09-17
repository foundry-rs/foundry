---
forge: patch
---

Symbolic tests fail closed again when `gasleft()` flows into a counterexample model or into call input, and a symbolic counterexample that does not replay is reported as an incomplete run without a user-facing counterexample.
