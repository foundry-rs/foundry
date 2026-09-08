---
chisel: patch
---

Fixed `!load`/`!load latest` trusting a cached session file's own (possibly missing, `null`, or stale) `id` field instead of the identifier actually used to locate the file, which could panic Chisel on a hand-edited or corrupted session file.
