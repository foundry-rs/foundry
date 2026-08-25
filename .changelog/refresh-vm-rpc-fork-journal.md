---
foundry-cheatcodes: patch
foundry-evm-core: patch
forge: patch
---

Fixed state-mutating `vm.rpc` calls such as `anvil_setCode` leaving already-loaded fork accounts stale during the same execution.
