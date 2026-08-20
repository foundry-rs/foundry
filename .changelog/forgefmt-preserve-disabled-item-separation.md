---
forge-fmt: patch
foundry-common: patch
---

Preserved source blank lines between items inside `forgefmt: disable-start`/`disable-end` regions instead of inserting the formatter's item separation; line-based directives such as `disable-line` keep the isolation break.
