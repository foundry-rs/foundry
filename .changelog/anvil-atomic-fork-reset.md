---
anvil: patch
---

Made fork resets atomic so failed or concurrent resets cannot expose partially updated chain state,
and reject resets that would change the node's fixed execution-network family.
