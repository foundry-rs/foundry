---
anvil: patch
---

Fixed a race between mining and chain-height-mutating calls: `anvil_rollback`/`anvil_reorg` could unwind the chain while a concurrent multi-block `evm_mine_detailed` was still running, since only individual block mining took the mining lock. This surfaced as either a confusing `BlockNotFound` mid-mine, or an "attempt to subtract with overflow" panic in `evm_mine_detailed`'s post-mining block lookup. `rollback` now holds the same lock mining does, and `evm_mine_detailed` no longer assumes the chain height it reads right after mining still covers every block it just mined.
