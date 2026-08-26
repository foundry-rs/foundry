---
forge: patch
cast: patch
anvil: patch
chisel: patch
foundry-config: patch
---

Sped up `foundry.toml` resolution by parsing and transforming the file once per load instead of once per merged section.
