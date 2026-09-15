---
forge: minor
cast: patch
---

Added `decode_external_storage` (and `forge test --decode-external-storage`), which decodes the storage slots of contracts outside the local project in `vm.getStateDiff()` output using verified block explorer sources.
