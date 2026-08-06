---
cast: patch
---

Fixed `cast abi-encode-event` for tuple parameters, including structs that contain external function pointers. Indexed reference parameters now hash the special in-place encoding defined for indexed event parameters, so their topics match the topics emitted by Solidity.
