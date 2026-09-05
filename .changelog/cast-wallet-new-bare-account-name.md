---
cast: patch
---

`cast wallet new <name>` now saves a named keystore to the default directory, matching `cast wallet import <name>`. Arguments that resolve to an existing path, or that fail to resolve for any reason other than not existing, keep reporting an error.
