---
cast: minor
---

Use block access lists to restore transaction prestate in `cast run` on supported Ethereum blocks, skipping earlier transaction replay with automatic fallback. Mismatched execution chain IDs or hardfork rules, ambiguous storage resets, and upstream blocks at or before an Anvil endpoint's fork block use replay. Preserve node-reported hardforks and beacon-root system updates when restoring debug prestate. `--prestate-tracer` takes priority, while `--quick` and remote tracing keep their existing execution modes. Reduce BAL prestate preparation overhead by consuming storage changes directly and reusing parent account reads.
