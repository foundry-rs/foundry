---
forge: minor
cast: patch
---

Added `decode_external_storage` (and `forge test --decode-external-storage`), which decodes the storage slots of contracts outside the local project in `vm.getStateDiff()` output. Verified sources come from the same Sourcify and block explorer path that decodes traces.
