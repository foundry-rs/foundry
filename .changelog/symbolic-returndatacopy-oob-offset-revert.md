---
forge: patch
foundry-evm-symbolic: patch
---

Fixed symbolic `RETURNDATACOPY` to explore both valid and out-of-bounds ranges and to clear return data when an out-of-bounds copy reverts.
