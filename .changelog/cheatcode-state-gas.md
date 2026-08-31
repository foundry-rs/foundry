---
forge: minor
---

Added `gasStateUsed` to the `Vm.Gas` values returned by `lastCallGas` and `lastFrameGas` while preserving existing gas snapshot totals, including refund-adjusted isolated snapshots.
