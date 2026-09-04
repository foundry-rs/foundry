---
forge: patch
---

`arbitrary-send-erc20` now analyzes modifier bodies, so a `transferFrom` with an arbitrary `from` hidden inside a modifier is flagged the same way it already is when written inline.
