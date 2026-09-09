---
forge: patch
---

Fixed a debugger memory-pane arithmetic overflow (the unfixed half of #6472) when a step's buffer offset or length saturates to `usize::MAX`.
