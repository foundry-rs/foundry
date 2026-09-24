---
forge-lint: patch
---

Fixed the `arbitrary-send-erc20` lint failing to detect a repeated flash-loan repayment pull inside a loop: a single `onFlashLoan` callback minted before a loop could license every `transferFrom` the (single-pass) loop body happened to contain, even though that same sink re-executes every iteration against the one license at runtime.
