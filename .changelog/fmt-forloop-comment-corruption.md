---
forge: patch
---

Fixed `forge fmt` silently corrupting `for` loops that have a trailing `//` comment on or after the header's closing brace, which could destroy source code with no error or warning.
