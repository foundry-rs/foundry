# Usage of unsafe cheatcodes

**Severity**: `Info`
**ID**: `unsafe-cheatcode`

## What it does

Reports calls to `ffi`, `readFile`, `readLine`, `writeFile`, `writeLine`, `removeFile`,
`closeFile`, `setEnv`, or `deriveKey`. Unrelated methods with these names may also be flagged.

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
