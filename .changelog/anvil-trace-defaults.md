---
anvil: patch
---

Fixed Anvil's `trace_call` returning call frames when no trace types are requested, `trace_filter` searching from genesis when `fromBlock` is omitted, and `trace_callMany` defaulting to the pending block instead of latest.
