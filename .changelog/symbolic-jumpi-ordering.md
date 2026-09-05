---
forge: patch
---

Fix symbolic execution rejecting or erroring on a `JUMPI` whose destination is garbage but whose condition is falsy, by only validating the jump destination when the branch is actually taken (matching the concrete EVM's behavior).
