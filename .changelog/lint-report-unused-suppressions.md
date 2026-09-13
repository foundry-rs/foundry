---
forge: minor
foundry-common: minor
---

Added `forge lint --report-unused-suppressions`, which reports inline `// forge-lint: disable-*` directives that did not suppress any diagnostic during the run, including individual lint IDs within a multi-ID directive.
