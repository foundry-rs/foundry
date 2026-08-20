---
foundry-cheatcodes: patch
foundry-evm-core: patch
foundry-evm: patch
forge-verify: patch
---

Replaced the bespoke chain-context capture/apply methods on `FoundryEvmFactory`/`NestedEvm` with direct access through revm's native `ContextTr::chain`/`chain_mut`, removing Monad-only methods from those generic traits.
