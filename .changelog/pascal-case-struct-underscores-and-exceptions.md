---
forge: patch
---

Fixed the `pascal-case-struct` lint to preserve leading/trailing underscores (matching `mixed-case-*` and `screaming-snake-case`) and to respect the configured `mixed_case_exceptions` acronym allowlist.
