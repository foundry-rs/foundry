---
forge: patch
cast: patch
anvil: patch
chisel: patch
foundry-config: patch
---

Sped up config resolution in projects with large `lib` trees by no longer auto-detecting remappings for a path layout that discards them.
