---
anvil: patch
---

Fixed loading state when the fork head is selected: dumped blocks and transactions at or below that head are no longer inserted into local storage, either by hash or by number. Block-by-hash, block-by-number, transaction-by-hash, and transaction-by-block-and-index queries consistently use the fork's live chain for that range.
