---
anvil: patch
---

Fixed a race between multi-block mining and chain-height-mutating calls. `evm_mine`, `evm_mine_detailed` and `anvil_mine` re-took the mining lock once per block, so anything that rewinds the chain could land between two blocks of a run that was still in flight: `evm_revert` and `anvil_loadState` over RPC, and `anvil_rollback`/`anvil_reorg` for direct `EthApi` callers. It surfaced as an "attempt to subtract with overflow" panic in `evm_mine_detailed`, whose post-mining lookup assumes the height it reads still covers every block it just mined. Those calls now hold the mining lock for the whole run, and `evm_mine`'s requested next-block timestamp is set under that lock so another block producer cannot consume it first. As a consequence, transactions submitted while a multi-block mine is running are no longer admitted to the pool mid-run, so every block of the run is drawn from the pool as it stood when the run started.
