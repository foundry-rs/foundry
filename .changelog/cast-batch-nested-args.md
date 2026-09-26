---
cast: patch
---

Fixed `cast batch-send` and `cast batch-mktx` to correctly parse array and tuple arguments containing commas, such as `[1,2]` and `(7,hello)`.
