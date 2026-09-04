---
foundry-evm: patch
---

Fixed `vm.deleteStateSnapshot`/`vm.deleteStateSnapshots` silently returning `false`/no-op for a snapshot taken before the current fuzz run's first mutating cheatcode call (e.g. a snapshot taken in `setUp()`), even though the snapshot genuinely exists.
