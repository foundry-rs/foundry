---
anvil: patch
---

Fixed `safe` and `finalized` block tags returning null on shallow chains started with a non-zero `--block-number` by falling back to the genesis block.
