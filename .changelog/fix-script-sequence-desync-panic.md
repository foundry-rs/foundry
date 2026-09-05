---
forge: patch
---

`forge script`'s broadcast-sequence loading no longer panics when a deployment's broadcast file and its sensitive-cache counterpart have a mismatched number of entries (e.g. from an interrupted `save()`); it now errors with a clear message instead.
