---
forge: patch
foundry-evm: patch
foundry-cheatcodes: patch
---

Fixed `vm.skip` in `setUp` being reported as a failure when an earlier revert was caught before the skip.
