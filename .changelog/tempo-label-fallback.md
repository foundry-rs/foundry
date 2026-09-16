---
foundry-evm: patch
forge: patch
forge-script: patch
cast: patch
chisel: patch
---

Decode Tempo token names up to 256 bytes in trace labels, falling back to TIP20 for larger, malformed, or unreadable names instead of panicking or displaying storage metadata.
