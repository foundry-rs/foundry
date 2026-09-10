---
anvil: patch
---

Fix `eth_sendTransaction`/`eth_sendRawTransaction` stacking replacement transactions in the
queued (future-nonce) pool instead of removing the one they replace, and fix
`anvil_dropTransaction` silently no-op'ing on a queued transaction.
