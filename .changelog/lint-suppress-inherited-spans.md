---
forge: patch
---

`forge lint` inline directives now suppress findings reported in an inherited base contract or a dependency, where previously only project-wide `exclude_lints` worked.
