---
forge: patch
---

Fixed symbolic `vm.expectCall` to merge duplicate non-counted registrations additively (matching concrete's documented idiom) and to reject duplicate counted registrations, instead of silently pushing a structurally-unreachable second entry.
