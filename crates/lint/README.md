# Linter (`lint`)

Solidity linter for identifying potential errors, vulnerabilities, gas optimizations, and style guide violations.
It helps enforce best practices and improve code quality within Foundry projects.

## Supported Lints

`forge-lint` includes rules across several categories:

- **High Severity:**
  - `incorrect-exp`: Flags `^` (bitwise xor) used where `**` (exponentiation) was likely intended.
  - `incorrect-shift`: Warns against shift operations where operands might be in the wrong order.
  - `unchecked-call`: Low-level calls should check the success return value.
  - `erc20-unchecked-transfer`: ERC20 `transfer` and `transferFrom` calls should check the return value.
  - `arbitrary-send-erc20`: Flags `transferFrom`/`safeTransferFrom` calls whose `from` argument is not provably `msg.sender` or `address(this)`.
  - `arbitrary-send-erc20-permit`: Flags arbitrary `transferFrom` calls preceded by a covering `permit`; on non-permit tokens with a fallback (e.g. WETH) the permit silently succeeds and previously-approved tokens can be drained.
  - `controlled-delegatecall`: Flags `delegatecall` calls whose target is not provably trusted.
  - `encode-packed-collision`: Flags `abi.encodePacked()` calls with multiple dynamic-type arguments (`string`, `bytes`, dynamic arrays) that can produce hash collisions.
  - `function-selector-collision`: Flags colliding selectors between a proxy and the statically typed implementation API targeted by its fallback.
  - `rtlo`: Flags Unicode bidirectional override characters ("Trojan Source", CVE-2021-42574) that can hide malicious code.
  - `reentrancy-eth`: Flags uncapped ETH-transferring low-level calls followed by writes to state that was read before the call.
  - `unprotected-initializer`: Upgradeable initializers should not be callable on the implementation contract.
- **Medium Severity:**
  - `assert-state-change`: Flags state-modifying expressions inside `assert()` arguments.
  - `boolean-cst`: Flags misuse of boolean constants.
  - `dangerous-unary-operator`: Flags an assignment whose `=` is fused to a unary operator (`=-`, `=~`), e.g. `x =- 1`, which parses as `x = -1` instead of the intended compound `x -= 1`.
  - `divide-before-multiply`: Warns against performing division before multiplication in the same expression, which can cause precision loss.
  - `incorrect-erc20-interface`: Flags ERC20 interfaces and implementations with non-compliant function signatures.
  - `incorrect-erc721-interface`: Flags ERC721 interfaces and implementations with non-compliant function signatures.
  - `incorrect-strict-equality`: Dangerous strict equality check on an externally-influenced value (ETH balance, ERC-20 balance).
  - `mapping-deletion`: `delete` on a value containing a mapping does not clear the mapping.
  - `reentrancy-no-eth`: Flags non-ETH external calls followed by writes to state that was read before the call.
  - `tautological-compare`: Comparing an expression with itself is always true or false.
  - `tx-origin`: Flags use of `tx.origin` in authorization-like predicates.
  - `uninitialized-local`: Local variable is read before being explicitly initialized.
  - `uninitialized-state`: State variable is read in functions but never written, so it always returns its zero-value default.
  - `unsafe-typecast`: Typecasts that can truncate values should be checked.
  - `unused-return`: Return value of an external call is not used.
  - `locked-ether`: Contracts that can receive ETH but have no mechanism to send it out.
  - `non-reentrant-not-first`: `nonReentrant` should be the first modifier on guarded entry points.
  - `weak-prng`: Flags randomness-like expressions derived from predictable on-chain values.
- **Low Severity:**
  - `block-timestamp`: Warns when `block.timestamp` is used in a comparison, as it may be manipulated by validators.
  - `calls-loop`: External calls inside loops can cause denial-of-service if a call reverts or exhausts gas.
  - `delegatecall-loop`: Payable functions should not use `delegatecall` inside a loop.
  - `deprecated-oz-function`: OpenZeppelin deprecated `SafeERC20.safeApprove` (use `safeIncreaseAllowance` / `safeDecreaseAllowance`) and `AccessControl._setupRole` (use `_grantRole`).
  - `empty-block`: Flags regular functions with an empty body; constructors, `receive`/`fallback`, `virtual` functions, functions with modifiers and value-less `payable` functions are exempt.
  - `incorrect-modifier`: Modifiers should not be able to finish without executing `_` or reverting.
  - `missing-events-access-control`: Access control changes should emit events.
  - `missing-zero-check`: Address parameter is used in a state write or value transfer without a zero-address check.
  - `reentrancy-events`: Events emitted after external calls can be reordered or fabricated by a reentrant callee and mislead off-chain consumers.
  - `return-bomb`: External calls with a gas limit should not consume unbounded return data.
  - `solmate-safe-transfer-lib`: solmate's released `SafeTransferLib` does not check that the token has code, so token operations against a token-less address succeed silently.
- **Informational / Style Guide:**
  - `boolean-equal`: Boolean comparisons to constants should be simplified.
  - `too-many-digits`: Numeric literals with 5+ consecutive zeros are error-prone.
  - `pascal-case-struct`: Flags for struct names not adhering to `PascalCase`.
  - `mixed-case-function`: Flags for function names not adhering to `mixedCase`.
  - `mixed-case-variable`: Flags for mutable variable names not adhering to `mixedCase`.
  - `screaming-snake-case-const`: Flags for `constant` variable names not adhering to `SCREAMING_SNAKE_CASE`.
  - `screaming-snake-case-immutable`: Flags for `immutable` variable names not adhering to `SCREAMING_SNAKE_CASE`.
  - `unused-import`: Unused imports should be removed.
  - `unaliased-plain-import`: Use named imports `{A, B}` or alias `import ".." as X`.
  - `named-struct-fields`: Prefer initializing structs with named fields.
  - `unsafe-cheatcode`: Usage of unsafe cheatcodes that can perform dangerous operations.
  - `multi-contract-file`: Prefer having only one contract, interface, or library per file.
  - `interface-file-naming`: Interface file names should be prefixed with `I`.
  - `interface-naming`: Interface names should be prefixed with `I`.
  - `pragma-inconsistent`: Flags projects whose source files declare different Solidity pragma version requirements.
  - `redundant-base-constructor-call`: Flags explicit empty base-constructor arguments (e.g. `is A()`) when the base requires no arguments.
  - `missing-inheritance`: Flags contracts that implement every external function of an interface without explicitly inheriting from it.
  - `low-level-calls`: Direct use of low-level calls should be avoided.
  - `event-fields`: `address` event parameters should be `indexed` for efficient log filtering.
  - `unused-error`: Custom error declarations that are never referenced should be removed.
  - `literal-instead-of-constant`: A literal value repeated inside a contract should be a named constant.
  - `function-init-state`: State variable initializers run before the constructor; depending on a non-pure function or another state variable there observes partial state.
  - `internal-function-used-once`: Internal functions referenced exactly once can usually be inlined into their caller.
  - `cyclomatic-complexity`: functions with a cyclomatic complexity above 11 should be split into smaller functions.
  - `incorrect-using-for`: `using ... for` directives naming a library with no function applicable to the type attach nothing and should be fixed or removed.
  - `modifier-used-only-once`: Modifiers invoked by exactly one function can usually be inlined as checks in that function.
- **Gas Optimizations:**
  - `asm-keccak256`: Recommends using inline assembly for `keccak256` for potential gas savings.
  - `cache-array-length`: Recommends caching storage dynamic array lengths used in `for` loop conditions.
  - `costly-loop`: Flags storage variable writes inside loops; accumulate into a local variable and write once after the loop instead.
  - `could-be-immutable`: Recommends declaring constructor-only state variables as `immutable`.
  - `could-be-constant`: Recommends declaring never-written state variables with a compile-time-constant initializer as `constant`.
  - `custom-errors`: Recommends using custom errors instead of strings and plain reverts for potential gas savings.
  - `external-function`: `public` functions never called internally should be declared `external` to avoid copying reference-type arguments into memory.
  - `unused-state-variables`: State variables that are never used should be removed.
  - `var-read-using-this`: Reads of state variables (or other `view`/`pure` functions) via `this` cause an unnecessary `STATICCALL`; access them directly.
  - `write-after-write`: Flags storage variables written consecutively without the first value ever being read; only the final write is needed.
- **Code Size:**
  - `unwrapped-modifier-logic`: Recommends wrapping modifier logic to reduce contract code size.

## Configuration

The behavior of the `SolidityLinter` can be customized with the following options:

| Option              | Default | Description                                                                                                            |
| ------------------- | ------- | ---------------------------------------------------------------------------------------------------------------------- |
| `with_severity`     | `None`  | Filters active lints by their severity (`High`, `Med`, `Low`, `Info`, `Gas`, `CodeSize`). `None` means all severities. |
| `with_lints`        | `None`  | Specifies a list of `SolLint` instances to include. Overrides severity filter if a lint matches.                       |
| `without_lints`     | `None`  | Specifies a list of `SolLint` instances to exclude, even if they match other criteria.                                 |
| `with_description`  | `true`  | Whether to include the lint's description in the diagnostic output.                                                    |
| `with_json_emitter` | `false` | If `true`, diagnostics are output in rustc-compatible JSON format; otherwise, human-readable text.                     |

## Contributing

Check out the [foundry contribution guide](https://github.com/foundry-rs/foundry/blob/master/CONTRIBUTING.md).

Guidelines for contributing to `forge lint`:

### Opening an issue

1. Create a short concise title describing an issue.
   - Bad Title Examples
     ```text
     Forge lint does not work
     Forge lint breaks
     Forge lint unexpected behavior
     ```
   - Good Title Examples
     ```text
     Forge lint does not flag incorrect shift operations
     ```
2. Fill in the issue template fields that include foundry version, platform & component info.
3. Provide the code snippets showing the current & expected behaviors.
4. If it's a feature request, specify why this feature is needed.
5. Besides the default label (`T-Bug` for bugs or `T-feature` for features), add `C-forge` and `Cmd-forge-fmt` labels.

### Fixing A Bug

1. Specify an issue that is being addressed in the PR description.
2. Add a note on the solution in the PR description.
3. Add a test case to `lint/testdata` that specifically demonstrates the bug and is fixed by your changes. Ensure all tests pass.

### Developing a New Lint Rule

Check the [dev docs](../../docs/dev/lintrules.md) for a full implementation guide.
