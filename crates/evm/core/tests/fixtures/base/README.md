# Base replay fixtures

These fixtures contain the minimum parent state and wire transaction bytes needed for deterministic
Base EVM replay. They are captured once from public Base RPC data and do not contact RPC endpoints
during tests.

`jovian-transfer.json`, `azul-transfer.json`, and `beryl-transfer.json` were captured from Base
mainnet blocks `0x2bf82ef`, `0x2cec52f`, and `0x2f4a86c` on 2026-08-05. Raw transactions came from
`debug_getRawTransaction`; account balances, nonces, and L1BlockInfo storage were read at each
parent block; expected execution fields came from `eth_getTransactionReceipt`.

Fixtures intentionally include sampled parent state rather than a full state trie. Tests may assert
the transaction result, gas, logs, fee-vault credits, and touched-account deltas, but must not claim
the canonical block state or receipt root unless the complete block state and transaction list are
included.
