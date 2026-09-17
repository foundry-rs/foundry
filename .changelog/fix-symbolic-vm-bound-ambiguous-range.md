---
forge: patch
foundry-evm-symbolic: patch
---

Fixed symbolic `vm.bound`/`vm.bound` (int256) leaving the path's constraints inconsistent with a chosen `Failure` outcome when the input's range membership was ambiguous, so later solves against that path could no longer be trusted to reflect the value actually being out of range.
