---
anvil: minor
---

Anvil now attempts to prefill its fork cache from the fork block's block access list, reducing remote state requests when the provider supports it. Use `--no-bal` to disable this behavior.
