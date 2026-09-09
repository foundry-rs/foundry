---
forge: patch
---

Fixed `forge script --broadcast` recording the wrong transaction hash against a script call when
concurrently-sent transactions from a batch confirmed out of submission order, corrupting the
persisted broadcast log used by `--resume` and verification.
