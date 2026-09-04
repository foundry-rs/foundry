---
forge: patch
---

Fixed `missing-zero-check` treating any `require`/`assert`/`if` predicate that merely mentions the parameter as a valid zero-address guard, even when the predicate never actually compares it against zero (e.g. `require(newOwner != address(this))`, `require(whitelisted[newOwner])`). The lint now only accepts a genuine `!= address(0)`-shaped comparison (through `&&`/`||`/negation) as a guard.
