---
cast: minor
---

Use block access lists to restore transaction prestate in `cast run` on supported Ethereum blocks, skipping earlier transaction replay with automatic fallback. Mismatched execution chain IDs, ambiguous storage resets, and upstream blocks at or before an Anvil endpoint's fork block use replay. `--prestate-tracer` takes priority, while `--quick` and remote tracing keep their existing execution modes.
