# Linter (`lint`)

Solidity linter for identifying potential errors, vulnerabilities, gas optimizations, and style guide violations.
It helps enforce best practices and improve code quality within Foundry projects.

## Supported Lints

`forge-lint` includes rules across several categories:

- **High Severity:**
  - `incorrect-shift`: Warns against shift operations where operands might be in the wrong order.
- **Medium Severity:**
  - `divide-before-multiply`: Warns against performing division before multiplication in the same expression, which can cause precision loss.
- **Informational / Style Guide:**
  - `pascal-case-struct`: Flags for struct names not adhering to `PascalCase`.
  - `mixed-case-function`: Flags for function names not adhering to `mixedCase`.
  - `mixed-case-variable`: Flags for mutable variable names not adhering to `mixedCase`.
  - `screaming-snake-case-const`: Flags for `constant` variable names not adhering to `SCREAMING_SNAKE_CASE`.
  - `screaming-snake-case-immutable`: Flags for `immutable` variable names not adhering to `SCREAMING_SNAKE_CASE`.
- **Gas Optimizations:**
  - `asm-keccak256`: Recommends using inline assembly for `keccak256` for potential gas savings.

## Configuration

The behavior of the `SolidityLinter` can be customized with the following options:

| Option              | Default | Description                                                                                                |
| ------------------- | ------- | ---------------------------------------------------------------------------------------------------------- |
| `with_severity`     | `None`  | Filters active lints by their severity (`High`, `Med`, `Low`, `Info`, `Gas`). `None` means all severities. |
| `with_lints`        | `None`  | Specifies a list of `SolLint` instances to include. Overrides severity filter if a lint matches.           |
| `without_lints`     | `None`  | Specifies a list of `SolLint` instances to exclude, even if they match other criteria.                     |
| `with_description`  | `true`  | Whether to include the lint's description in the diagnostic output.                                        |
| `with_json_emitter` | `false` | If `true`, diagnostics are output in rustc-compatible JSON format; otherwise, human-readable text.         |

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
