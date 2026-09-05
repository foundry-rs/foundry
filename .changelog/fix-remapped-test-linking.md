---
forge: patch
foundry-common: patch
---

Fixed dynamic test linking and incremental rebuilds for contracts imported through source remappings, including inherited test contracts. Run `forge test --force` once to rebuild artifacts cached by an affected version.
