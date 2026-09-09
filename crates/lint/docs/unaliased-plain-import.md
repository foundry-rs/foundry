# Unaliased plain import

**Severity**: `Info`
**ID**: `unaliased-plain-import`

Flags `import "path";` statements that pull in every top-level symbol from another file without
an alias.

## What it does

Reports plain imports of the form `import "path";`. Suggests using either named imports
(`import { A, B } from "path"`) or an aliased import (`import "path" as X`).

## Why is this bad?

Plain imports pollute the importing file's namespace and make the source of each symbol
non-obvious. Named or aliased imports make the dependency surface explicit and reduce the chance
of accidental name collisions.

## Example

### Bad

```solidity
import "./Lib.sol";
```

### Good

```solidity
import { Foo, Bar } from "./Lib.sol";
// or
import "./Lib.sol" as Lib;
```
