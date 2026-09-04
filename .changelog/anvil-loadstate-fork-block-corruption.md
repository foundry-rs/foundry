---
anvil: patch
---

Fix `anvil_loadState` on a forked node silently replacing the fork's own canonical blocks below its head number with unrelated blocks from the loaded dump, producing a chain with mismatched `parentHash` links. Loading a state dump on a forked node no longer lets dumped blocks or transactions numbered at or below the fork's head claim that block number's canonical slot; only the fork's own real blocks occupy those numbers.
