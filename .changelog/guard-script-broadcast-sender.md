---
forge: minor
foundry-cheatcodes: minor
foundry-evm: minor
---

Reject `msg.sender` reads in the main script's broadcasting frame when the caller differs from the broadcast sender. This guard follows `script_execution_protection` and can be disabled with that setting.
