# Linter (`lint`)

Solidity linter for identifying potential errors, vulnerabilities, gas optimizations, and style guide violations.
It helps enforce best practices and improve code quality within Foundry projects.

## Architecture

The `forge-lint` system operates by analyzing Solidity source code through a dual-pass system:

1. **Parsing**: Solidity source files are parsed into an Abstract Syntax Tree (AST) using `solar`. This AST represents the syntactic structure of the code.
2. **HIR Generation**: The AST is then lowered into a High-level Intermediate Representation (HIR) that includes type information and semantic analysis.
3. **Early Lint Passes**: The `EarlyLintVisitor` traverses the AST, invoking registered "early lint passes" (`EarlyLintPass` implementations) for syntax-level checks.
4. **Late Lint Passes**: The `LateLintVisitor` traverses the HIR, invoking registered "late lint passes" (`LateLintPass` implementations) for semantic analysis.
5. **Emitting Diagnostics**: If a lint pass identifies a violation, it uses the `LintContext` to emit a diagnostic (either `warning` or `note`) that pinpoints the issue. Lints can also provide code fix suggestions through the `Suggestion` API, which integrates with solar's diagnostic system to support different applicability levels.

### Key Components

- **`Linter` Trait**: Defines a generic interface for linters. `SolidityLinter` is the concrete implementation tailored for Solidity.
- **`Lint` Trait & `SolLint` Struct**:
  - `Lint`: A trait that defines the essential properties of a lint rule, such as its unique ID, severity, description, and an optional help message/URL.
  - `SolLint`: A struct implementing the `Lint` trait, used to hold the metadata for each specific Solidity lint rule.
- **`EarlyLintPass<'ast>` Trait**: Lints that operate directly on AST nodes implement this trait. It contains methods (like `check_expr`, `check_item_function`, etc.) called by the AST visitor.
- **`LateLintPass<'hir>` Trait**: Lints that require type information and semantic analysis implement this trait. It contains methods (like `check_contract`, `check_function`, etc.) called by the HIR visitor.
- **`LintContext<'s>`**: Provides contextual information to lint passes during execution, such as access to the session for emitting diagnostics and methods for emitting suggestions.
- **`EarlyLintVisitor<'a, 's, 'ast>`**: The visitor that traverses the AST and dispatches checks to the registered `EarlyLintPass` instances.
- **`LateLintVisitor<'a, 's, 'hir>`**: The visitor that traverses the HIR and dispatches checks to the registered `LateLintPass` instances.
- **`Suggestion` Struct**: Represents code fix suggestions with different kinds (fix or example) and applicability levels, integrated with solar's diagnostic system.

## Developing a new lint rule

We recommend you start by writing out some Solidity code that you want to trigger a lint in [`crates/lint/testdata`](https://github.com/foundry-rs/foundry/tree/master/crates/lint/testdata). Name the file after your lint rule.

Next, choose whether you want an [early or late lint pass](#choosing-between-early-and-late-passes). If your lint is early, you can use use [Solar](https://github.com/paradigmxyz/solar) to dump the AST and find the patterns you need to match on in your lint code using `solar -Zdump=ast crates/lint/testdata/<file.sol>`. If your lint is late, you can use `solar -Zdump=hir crates/lint/testdata/<file.sol>`.

1. Specify an issue that is being addressed in the PR description.
2. In your PR:

- Create a static `SolLint` instance using the `declare_forge_lint!` to define its metadata.
  ```rust
  declare_forge_lint!(
      MIXED_CASE_FUNCTION,                      // The Rust identifier for this SolLint static
      Severity::Info,                           // The default severity of the lint
      "mixed-case-function",                    // A unique string ID for configuration/CLI
      "function names should use mixedCase"     // A brief description
  );
  // Note: The macro automatically generates a help link to the Foundry book
  ```

- Register the pass struct and the lint using `register_lints!` in the `mod.rs` of its corresponding severity category. Specify the pass type (`early`, `late`, or both). Note that a single pass can handle multiple lints:
  ```rust
  register_lints!(
    (PascalCaseStruct, early, (PASCAL_CASE_STRUCT)),
    (MixedCaseVariable, early, (MIXED_CASE_VARIABLE)),
    (MixedCaseFunction, early, (MIXED_CASE_FUNCTION)),
    (ScreamingSnakeCase, early, (SCREAMING_SNAKE_CASE_CONSTANT, SCREAMING_SNAKE_CASE_IMMUTABLE)),
    (AsmKeccak256, late, (ASM_KECCAK256))
  );
  // The macro automatically generates the pass structs and helper functions
  ```

- Implement the appropriate trait logic (`EarlyLintPass` or `LateLintPass`) for your lint. Do it in a new file within the relevant severity module (e.g., `src/sol/med/my_new_lint.rs`).

### Choosing Between Early and Late Passes

- **Use `EarlyLintPass`** for:
  - Syntax-level checks (naming conventions, formatting)
  - Simple pattern matching that doesn't require type information
  - Lints that can be determined from the AST alone

- **Use `LateLintPass`** for:
  - Semantic analysis requiring type information
  - Cross-reference checks between different parts of the code
  - Complex patterns that need to understand the actual behavior
  - Avoiding false positives through type-aware analysis

### Providing Code Fix Suggestions

Lints can provide actionable code fix suggestions using the `emit_with_suggestion` method. The `Suggestion` API integrates with solar's diagnostic system and supports different applicability levels:

```rust
use solar::interface::diagnostics::Applicability;

// Example: Suggesting a machine-applicable fix
cx.emit_with_suggestion(
    lint,
    node.span,
    Suggestion::fix(
        corrected_name,
        Applicability::MachineApplicable,
    )
    .with_desc("consider using")
);

// Example: Suggesting a fix with a specific span
cx.emit_with_suggestion(
    lint,
    node.span,
    Suggestion::fix(
        optimized_code,
        Applicability::MaybeIncorrect,
    )
    .with_desc("use inline assembly for gas optimization")
    .with_span(replacement_span)
);

// Example: Providing an example (non-applicable suggestion)
cx.emit_with_suggestion(
    lint,
    node.span,
    Suggestion::example("some example")
);
```

**Applicability Levels:**
- `MachineApplicable`: The suggestion can be applied automatically with high confidence
- `MaybeIncorrect`: The suggestion might not be correct in all cases and should be reviewed
- `HasPlaceholders`: The suggestion contains placeholders that need to be filled in
- `Unspecified`: No applicability specified

3. Add comprehensive tests in `lint/testdata/`:
   - Create `MyNewLint.sol` with various examples (triggering and non-triggering cases, edge cases).
   - If your test requires imports, add those files under `lint/testdata/auxiliary/` so that the ui runner doesn't lint them.
   - Generate the corresponding blessed file with the expected output.

### Testing a lint rule

Tests are located in the `lint/testdata/` directory. A test for a lint rule involves:

- A Solidity source file with various code snippets, some of which are expected to trigger the lint. Expected diagnostics must be indicated with either `//~WARN: description` or `//~NOTE: description` on the relevant line.
- corresponding `.stderr` (blessed) file which contains the exact diagnostic output the linter is expected to produce for that source file.

The testing framework runs the linter on the `.sol` file and compares its standard error output against the content of the `.stderr` file to ensure correctness.

- Run the following command to trigger the ui test runner:
  ```sh
  // using the default cargo cmd for running tests
  cargo test -p forge --test ui

  // using nextest
  cargo nextest run -p forge test ui
  ```

- If you need to generate / bless (re-generate) the output files:
  ```sh
  // using the default cargo cmd for running tests
  cargo bless-lints
  ```
