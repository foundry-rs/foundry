---
cast: patch
foundry-common: patch
---

`cast wallet new` no longer repeats the address and private key as a tab-separated stdout record in interactive terminals; the machine-readable record is still emitted when stdout is piped or redirected.
