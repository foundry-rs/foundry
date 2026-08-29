---
cast: patch
foundry-evm-core: patch
---

Made `cast run` decode blocks with `AnyNetwork`, so chains whose blocks carry non-standard transaction types, such as Arbitrum, Celo and unrouted OP-stack forks, can be replayed at all.
