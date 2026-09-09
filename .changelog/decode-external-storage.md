---
forge: minor
cast: patch
---

Added `decode_external_storage` (and `forge test --decode-external-storage`), which decodes the storage slots of contracts outside the local project in `vm.getStateDiff()` output. Verified sources come from the same Sourcify and block explorer path that decodes traces, so no API key is required, and are then compiled for a storage layout: proxies resolve to their implementation, lookups are deduplicated across parallel tests, and resolved layouts are cached on disk. Remapping paths from explorer sources are also no longer trusted to stay inside the checkout.
