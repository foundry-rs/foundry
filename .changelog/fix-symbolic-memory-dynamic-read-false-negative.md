---
forge: patch
foundry-evm-symbolic: patch
---

Fixed a false negative in symbolic execution's dynamic-offset memory reads: a write recorded past the tracked materialized region (including one dispatched as symbolic despite its offset being constant-evaluable) could silently read back as zero instead of its stored value, which could hide a genuine violation behind an apparent "Safe" result.
