# Unused import

**Severity**: `Info`
**ID**: `unused-import`

## What it does

Reports unused names in `import { A, B } from "..."` and unused namespace aliases from
`import "..." as X` or `import * as X from "..."`. Plain unaliased imports are not checked.

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
