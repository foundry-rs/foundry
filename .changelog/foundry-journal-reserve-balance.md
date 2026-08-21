---
foundry-cheatcodes: patch
foundry-evm-core: patch
foundry-evm: patch
---

Replaced the bespoke transaction-state capture/apply methods on `FoundryEvmFactory`/`NestedEvm` with direct access through revm's native `ContextTr::journal`/`journal_mut`, via a new `FoundryJournal` trait whose Monad reserve-balance-tracker methods are concrete and `#[cfg(feature = "monad")]`-gated, removing the last Monad-only methods and associated types from those generic traits.
