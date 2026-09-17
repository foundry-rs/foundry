---
anvil: patch
---

Fix `eth_feeHistory` returning `null` for `gasUsedRatio` on a zero-gas-limit block (e.g. via `evm_setBlockGasLimit(0)` or `--gas-limit 0`), which broke real clients including `cast`.
