---
chisel: patch
---

Fixed `chisel`'s autosave picking a session id from the cache directory's entry count instead of the highest existing numeric id, which could silently overwrite an existing saved session once any session had been deleted or renamed.
