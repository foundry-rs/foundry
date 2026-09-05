---
forge: patch
---

Fixed the symbolic engine (`--symbolic`) silently continuing past `RETURNDATACOPY` instead of reverting when the offset is already out of range and the size is a genuinely-symbolic value provably bounded to 0.
