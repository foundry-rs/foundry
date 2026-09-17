---
forge: minor
forge-lint: minor
---

Warn about `block.number` captures crossing `vm.roll` and `block.timestamp` captures crossing
`vm.warp` in tests, scripts, and helpers. The new lints recommend `vm.getBlockNumber()` and
`vm.getBlockTimestamp()` so test captures remain reliable with compiler optimizations enabled.

Avoid false positives across external library calls, exhausted loops, and mutually exclusive
helper returns.
