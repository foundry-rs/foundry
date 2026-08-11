---
forge: patch
---

Scoped ambiguous auto-detected transitive remappings to their owning dependencies. Root imports
retain a deterministic global fallback selected by shortest target path, then a `src` target, then
lexical path order.
