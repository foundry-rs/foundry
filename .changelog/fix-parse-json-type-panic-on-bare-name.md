---
forge: patch
foundry-cheatcodes: patch
---

Fixed `vm.parseJsonType`/`vm.parseTomlType` (and related type-resolving cheatcodes) panicking instead of returning the intended error when given a bare type/struct name.
