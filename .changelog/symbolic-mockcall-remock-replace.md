---
forge: patch
---

Fixed `vm.mockCall`/`vm.mockCalls` under `--symbolic` appending a duplicate mock instead of replacing the prior one when re-registering the same `(callee, value, calldata)`, diverging from concrete-mode's replace semantics.
