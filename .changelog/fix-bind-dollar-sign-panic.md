---
forge: patch
forge-sol-macro-gen: patch
---

Fixed `forge bind` panicking on a contract name containing `$`, which is a legal Solidity identifier character but not a legal Rust one.
