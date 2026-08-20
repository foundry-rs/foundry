---
forge-fmt: patch
---

Fixed `forge fmt` dropping the space after the opening brace of an empty contract or struct body holding only comments, crashing on a run of block comments at the end of a contract, struct, or enum body, and splitting such runs when `wrap_comments` is enabled.
