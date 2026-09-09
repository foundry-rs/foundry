---
forge: minor
forge-lint: minor
---

Replaced `block-number-across-roll` and `block-timestamp-across-warp` with
`environment-read-across-mutation`, covering invariant block and transaction reads across
Foundry environment setters, fork changes, and snapshot restoration. Update lint selections
and suppressions to the new ID.
