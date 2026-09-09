---
forge-verify: patch
---

Made `forge verify-bytecode` decode blocks with `AnyNetwork`, so contracts on chains whose blocks carry non-standard transaction types, such as Arbitrum and Celo, can be verified.
