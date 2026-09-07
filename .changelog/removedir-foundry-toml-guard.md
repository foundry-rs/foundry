---
forge: patch
---

`vm.removeDir` now refuses to target `foundry.toml` (or a directory containing it), matching the existing protection on `vm.writeFile`, `vm.writeFileBinary`, `vm.writeLine`, `vm.copyFile`, and `vm.removeFile`.
