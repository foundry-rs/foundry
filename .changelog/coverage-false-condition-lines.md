---
forge: patch
foundry-evm-coverage: patch
---

Fixed coverage marking executed `if` conditions as uncovered when only the false path runs, which could produce inconsistent LCOV line and branch counts.
