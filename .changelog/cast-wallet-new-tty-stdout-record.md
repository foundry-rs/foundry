---
cast: patch
foundry-common: patch
---

`cast wallet new` and `cast create2` no longer repeat the generated values as a tab-separated stdout record in interactive terminals; the machine-readable record is still emitted when stdout is piped or redirected.
