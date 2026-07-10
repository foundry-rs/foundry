# Usage of unsafe cheatcodes

**Severity**: `Info`
**ID**: `unsafe-cheatcode`

Flags use of Foundry cheatcodes that perform dangerous side effects (filesystem access, network
activity, environment variable reads, etc.) so they cannot slip into production code unnoticed.

## What it does

Reports calls to cheatcodes whose effects extend beyond the EVM sandbox or that bypass typical
test invariants. The flagged set follows the cheatcode's
[`Safety::Unsafe`](https://book.getfoundry.sh/cheatcodes) classification.

## Why is this bad?

Unsafe cheatcodes can read/write files, hit the network, or fork external state. They are
appropriate in tests with explicit intent but should not be added without review, and must
never end up in shipped contract code.

## Example

### Bad

```solidity
vm.writeFile("./out.txt", data);   // unsafe — writes to host filesystem
vm.envString("PRIVATE_KEY");       // unsafe — reads host environment
```

### Good

```solidity
// Use safe cheatcodes (vm.expectRevert, vm.prank, vm.warp, ...) and explicit
// inputs/fixtures instead of pulling state from the host environment.
```
