# Unaliased plain import

**Severity**: `Info`
**ID**: `unaliased-plain-import`

## What it does

Reports plain imports of the form `import "path";`. Suggests using either named imports
(`import { A, B } from "path"`) or an aliased import (`import "path" as X`).

## Why restrict this?

Plain imports pollute the importing file's namespace and make the source of each symbol
non-obvious. Named or aliased imports make the dependency surface explicit and reduce the chance
of accidental name collisions.

Plain imports may be intentional for a project's re-export or generated-code conventions.
Preserve them when changing the imported namespace would break consumers or obscure that intent.

## Example

```solidity
import "./Lib.sol";
```

Use instead:

```solidity
import { Foo, Bar } from "./Lib.sol";
// or
import "./Lib.sol" as Lib;
```
