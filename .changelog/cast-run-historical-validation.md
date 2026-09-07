---
cast: patch
---

Stopped `cast run` from re-validating mined transactions against the block gas limit and from requiring a parent beacon block root, which made BSC, Polygon and Scroll transactions unreplayable.
