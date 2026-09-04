---
forge: patch
forge-doc: patch
---

Fixed `forge doc` rewriting `](` sequences inside code fences and inline code spans as if they were markdown links, which corrupted Solidity syntax like `new address[](2)` on the generated documentation homepage. Also fixed a panic when a homepage README ends in an unclosed link target followed by a trailing backslash.
