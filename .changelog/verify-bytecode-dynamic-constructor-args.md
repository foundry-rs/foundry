---
forge: patch
---

Fixed `forge verify-bytecode` mis-encoding dynamic constructor arguments (`string`, `bytes`, dynamic arrays, and tuples containing them), which caused verification to always fail for contracts with such constructors.
