---
forge: patch
foundry-evm-symbolic: patch
---

Prove order and error bounds for unsigned rounding to constant multiples, and reuse those bounds for guarded fixed-point conversions, including both common ceiling-addition forms. Preserve bounds relative to both the original dividend and a safely shifted anchor, and recognize normalized multiplication and addition guards in unoptimized Solidity bytecode.
