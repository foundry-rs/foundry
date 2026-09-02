---
forge: patch
foundry-cheatcodes: patch
---

Kept the decoded revert reason in `expectRevert` mismatch errors when distinct payloads decode identically, appending the raw data instead of replacing the message with bare hex.
