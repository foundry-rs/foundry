---
anvil: patch
foundry-evm-core: patch
forge: patch
cast: patch
---

Fixed Tempo forks importing placeholder native balances from RPC, which could trigger extra native transfers and incorrect gas usage. Historical Anvil account-info reads now return balance, nonce, and code from the requested block, including when serving downstream forks.
