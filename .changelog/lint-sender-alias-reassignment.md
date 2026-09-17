---
forge: patch
forge-lint: patch
---

Stopped treating a local as a `msg.sender` alias once it is reassigned to another value, so
`missing-events-access-control` and `missing-events-arithmetic` no longer accept a check on the
reassigned local as an access-control guard.
