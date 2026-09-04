---
anvil: patch
---

Restored backwards compatibility for `anvil_loadState` / `--load-state` when a state dump file omits the `block` or `best_block_number` keys entirely (rather than carrying them with an explicit `null`). Both fields are documented as optional for backwards compatibility but were missing `#[serde(default)]`, making them hard-required by serde despite the deserializer being written to accept `None`.
