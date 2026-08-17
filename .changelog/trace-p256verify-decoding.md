---
forge: patch
cast: patch
---

Fixed trace decoding for the P256VERIFY precompile, which was skipped by a too-strict address check and displayed as an unrelated selector.
