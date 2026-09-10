---
forge: minor
forge-lint: minor
---

Replaced `block-number-across-roll` and `block-timestamp-across-warp` with
`environment-read-across-mutation`, covering invariant block and transaction reads across
Foundry environment setters, fork changes, and snapshot restoration. Update lint selections
and suppressions to the new ID. Diagnostics retain the setter name (for example, "across
`vm.warp`") and highlight its call as a secondary span alongside the original read.
Capture advice appears in a separate help message.
