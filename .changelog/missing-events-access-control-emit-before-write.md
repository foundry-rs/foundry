---
forge: patch
---

Fixed `missing-events-access-control` reporting a false positive when the `emit` documenting a
state change is written before the change itself, instead of after.
