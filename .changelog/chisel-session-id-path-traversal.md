---
chisel: patch
---

Reject a `chisel` session id containing a path separator (or `.`/`..`) in `!save`/`!load`/`chisel load`/`chisel view`, instead of concatenating it straight into `chisel-<id>.json`. An id like `x/../../../../../tmp/pwned` previously resolved clean outside the cache directory, so a mistyped or copy-pasted id could silently overwrite an arbitrary file the user could write to.
