---
forge: patch
foundry-evm-symbolic: patch
---

Fixed symbolic invalid jumps to revert only the current EVM call frame and leave untaken conditional jumps unaffected.
