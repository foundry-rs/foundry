---
cast: patch
---

Fix `cast from-wei` and `cast format-units` silently returning a garbage ~1.157e59 value for
negative input instead of the correct signed result.
