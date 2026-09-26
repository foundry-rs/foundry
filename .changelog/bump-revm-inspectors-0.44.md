---
anvil: patch
cast: patch
forge: patch
chisel: patch
---

Updated revm-inspectors to 0.44.0, which fixes Parity `trace_*` state diffs and VM traces, JS tracer BigInt compatibility, and opcode tracer step limits. JSON traces from `forge test` and `forge script` now include empty `bytecode` and `step_deltas` fields.
