# Linter (`lint`)

Solidity linter for identifying potential errors, vulnerabilities, gas optimizations, and style guide violations.
It helps enforce best practices and improve code quality within Foundry projects.

## Supported Lints

`forge-lint` includes rules across several categories:

*   **High Severity:**
    *   `incorrect-shift`: Warns against shift operations where operands might be in the wrong order.
*   **Medium Severity:**
    *   `divide-before-multiply`: Warns against performing division before multiplication in the same expression, which can cause precision loss.
*   **Informational / Style Guide:**
    *   `pascal-case-struct`: Flags for struct names not adhering to `PascalCase`.
    *   `mixed-case-function`: Flags for function names not adhering to `mixedCase`.
    *   `mixed-case-variable`: Flags for mutable variable names not adhering to `mixedCase`.
    *   `screaming-snake-case-const`: Flags for `constant` variable names not adhering to `SCREAMING_SNAKE_CASE`.
    *   `screaming-snake-case-immutable`: Flags for `immutable` variable names not adhering to `SCREAMING_SNAKE_CASE`.
*   **Gas Optimizations:**
    *   `asm-keccak256`: Recommends using inline assembly for `keccak256` for potential gas savings.

## Architecture

The `forge-lint` system operates by analyzing Solidity source code:

1.  **Parsing**: Solidity source files are parsed into an Abstract Syntax Tree (AST) using `solar-parse`. This AST represents the structure of the code.
2.  **AST Traversal**: The generated AST is then traversed using a Visitor pattern. The `EarlyLintVisitor` is responsible for walking through the AST nodes.
3.  **Applying Lint Passes**: As the visitor encounters different AST nodes (like functions, expressions, variable definitions), it invokes registered "lint passes" (`EarlyLintPass` implementations). Each pass is designed to check for a specific code pattern.
4.  **Emitting Diagnostics**: If a lint pass identifies a violation of its rule, it uses the `LintContext` to emit a diagnostic (either `warning` or `note`) that pinpoints the issue in the source code.

### Key Components

*   **`Linter` Trait**: Defines a generic interface for linters. `SolidityLinter` is the concrete implementation tailored for Solidity.
*   **`Lint` Trait & `SolLint` Struct**:
    *   `Lint`: A trait that defines the essential properties of a lint rule, such as its unique ID, severity, description, and an optional help message/URL.
    *   `SolLint`: A struct implementing the `Lint` trait, used to hold the metadata for each specific Solidity lint rule.
*   **`EarlyLintPass<'ast>` Trait**: Lints that operate directly on AST nodes implement this trait. It contains methods (like `check_expr`, `check_item_function`, etc.) called by the visitor.
*   **`LintContext<'s>`**: Provides contextual information to lint passes during execution, such as access to the session for emitting diagnostics.
*   **`EarlyLintVisitor<'a, 's, 'ast>`**: The core visitor that traverses the AST and dispatches checks to the registered `EarlyLintPass` instances.

## Configuration

The behavior of the `SolidityLinter` can be customized with the following options:

| Option              | Default | Description                                                                                                |
|---------------------|---------|------------------------------------------------------------------------------------------------------------|
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
3.  Add a test case to `lint/testdata` that specifically demonstrates the bug and is fixed by your changes. Ensure all tests pass.

### Developing a New Lint Rule

1. Specify an issue that is being addressed in the PR description.
2. In your PR:
    *   Implement the lint logic by creating a new struct and implementing the `EarlyLintPass` trait for it within the relevant severity module (e.g., `src/sol/med/my_new_lint.rs`).
    *   Declare your `SolLint` metadata using `declare_forge_lint!`.
    *   Register your pass and lint using `register_lints!` in the `mod.rs` of its severity category.
3. Add comprehensive tests in `lint/testdata/`:
    *   Create `MyNewLint.sol` with various examples (triggering and non-triggering cases, edge cases).
    *   Create `MyNewLint.stderr` with the expected output.

### Testing

Tests are located in the `lint/testdata` directory. A test for a lint rule involves:

 - A Solidity source file with various code snippets, some of which are expected to trigger the lint. Expected diagnostics must be indicated with either `//~WARN: description` or `//~NOTE: description` on the relevant line.
 - corresponding `.stderr` (blessed) file which contains the exact diagnostic output the linter is expected to produce for that source file.

The testing framework runs the linter on the `.sol` file and compares its standard error output against the content of the `.stderr` file to ensure correctness.

- Run the following commands to trigger the ui test runner:
  ```sh
  // using the default cargo cmd for running tests
  cargo test -p forge --test ui

  // using `nextest` for running tests
  cargo nextest run -p forge --test ui
  ```

- If you need to generate the blessed files:
  ```sh
  // using the default cargo cmd for running tests
  BLESS=1 cargo test -p forge --test ui

  // using `nextest` for running tests
  BLESS=1 cargo nextest run -p forge --test ui
  ```
