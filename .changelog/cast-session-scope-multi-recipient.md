---
cast: patch
---

Fixed `cast wallet session` / `cast keychain` scope parsing (`TARGET:SELECTOR@RECIPIENTS`) rejecting more than one comma-separated recipient address, which made it impossible to restrict a selector to an allowlist of multiple recipients.
