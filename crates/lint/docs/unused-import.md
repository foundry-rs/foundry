# Unused import

**Severity**: `Info`
**ID**: `unused-import`

Flags imported symbols (or whole import statements) whose imported names are not referenced
anywhere in the source file.

## What it does

Reports `import "..."`, `import "..." as X`, and `import { A, B } from "..."` statements where one
or more imported names are never used. This includes unused namespace imports (`import * as X`).

## Why is this bad?

Unused imports add noise, slow down compilation, can cause name collisions, and frequently
indicate dead code or stale refactors.

## Example

```solidity
import { A, B } from "./Lib.sol"; // B is never used

contract C {
    A internal a;
}
```

Use instead:

```solidity
import { A } from "./Lib.sol";

contract C {
    A internal a;
}
```
