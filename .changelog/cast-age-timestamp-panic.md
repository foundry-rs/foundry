---
cast: patch
---

Fixed `cast age` panicking when a block's timestamp falls outside the range Chrono can represent as a `DateTime`, instead of returning a clean error.
