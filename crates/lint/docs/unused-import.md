# Unused import

**Severity**: `Info`
**ID**: `unused-import`

Flags imported symbols (or whole import statements) whose imported names are not referenced
anywhere in the source unit.

## What it does

Reports `import "..."`, `import "..." as X`, and `import { A, B } from "..."` statements where one
or more imported names are never used. Symbols brought in via `import * as X` are tracked through
`X.member` accesses.

## Why is this bad?

Unused imports add noise, slow down compilation, can cause name collisions, and frequently
indicate dead code or stale refactors.

## Example

### Bad

```solidity
import { A, B } from "./Lib.sol"; // B is never used

contract C {
    A internal a;
}
```

### Good

```solidity
import { A } from "./Lib.sol";

contract C {
    A internal a;
}
```
