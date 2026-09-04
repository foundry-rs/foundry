---
forge-lint: patch
---

Fixed a false positive in `arbitrary-send-erc20` where a `permit`/`transferFrom` owner correlated through a numeric cast round-trip (e.g. `address(uint160(rawToken))`) was no longer recognized as the same variable, a regression from the `sol/analysis` consolidation. The peeling now only applies to casts that cannot truncate an address value, so amount/fee correlation in flash-loan repayment tracking stays conservative.
