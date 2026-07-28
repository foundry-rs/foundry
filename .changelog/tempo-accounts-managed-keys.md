---
cast: patch
forge: patch
---

Removed the separate `sessions.toml` registry. Cast and Forge temporary access-key commands now
create, resolve, expire, and retire keys through the canonical Tempo Accounts `store.json`.
