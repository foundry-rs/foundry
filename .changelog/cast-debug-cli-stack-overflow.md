---
cast: patch
---

Fixed a stack overflow when parsing `cast` arguments in debug builds on threads with a 2 MiB stack, such as unit test threads.
