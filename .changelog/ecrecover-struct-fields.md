---
forge: patch
---

Fixed the `ecrecover` lint ignoring low-`s` guards on signature struct fields such as `sig.s`.
