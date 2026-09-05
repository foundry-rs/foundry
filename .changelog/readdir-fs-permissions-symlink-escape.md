---
forge: patch
---

Stop `vm.readDir(path, maxDepth, followLinks: true)` from following a symlink out of the `fs_permissions` boundary and listing file/directory names outside it.
