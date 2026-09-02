---
forge-script: patch
---

Fixed a panic in `forge script` when a CREATE2 deployer call's input was shorter than the 32-byte salt prefix.
