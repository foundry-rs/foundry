---
forge: patch
foundry-evm-symbolic: patch
---

Fixed symbolic `vm.expectRevert` and `vm.expectPartialRevert` to reject a second registration before the pending expectation is consumed, matching concrete execution instead of silently overwriting the first expectation.
