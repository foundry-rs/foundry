---
forge: patch
foundry-evm: patch
foundry-evm-core: patch
---

Restored Forge test performance in Monad-enabled release builds by keeping per-opcode inspector hooks inline without changing system replay behavior.
