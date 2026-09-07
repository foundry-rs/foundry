---
forge: patch
foundry-common: patch
---

Fixed dynamic test linking and subsequent incremental rebuilds for contracts imported through source remappings, including inherited test contracts. After upgrading, run `forge test --force` once in each project with artifacts cached by an affected version.
