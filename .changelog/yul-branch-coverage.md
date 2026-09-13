---
forge: patch
foundry-evm-coverage: patch
---

Track both outcomes of assembly `if` statements in coverage reports so an executed condition with an untaken body does not fail LCOV branch consistency checks.
