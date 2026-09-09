# Usage of unsafe cheatcodes

**Severity**: `Info`
**ID**: `unsafe-cheatcode`

Flags use of Foundry cheatcodes classified as unsafe so their effects can receive deliberate review.

## What it does

Reports member calls named `ffi`, `readFile`, `readLine`, `writeFile`, `writeLine`, `removeFile`,
`closeFile`, `setEnv`, or `deriveKey`. This is a fixed name-based list: it does not resolve the
receiver to the cheatcode interface or derive its coverage from cheatcode safety metadata.

## Why restrict this?

Unsafe cheatcodes can interact with the host environment or introduce external dependencies into
tests. A project may restrict them for reproducibility or to limit side effects. They can be
appropriate in trusted tests and scripts; review the particular cheatcode before allowing it.

## Example

```solidity
string memory expected = vm.readFile("./expected-label.txt");
```

Use instead:

```solidity
// Keep a deterministic test fixture in the test source.
string memory expected = "test-label";
```
