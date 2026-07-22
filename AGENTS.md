# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Overview

Foundry is a fast, portable, modular toolkit for Ethereum application development,
written in Rust.

- `forge`: build, test, fuzz, debug, lint, and deploy Solidity contracts
- `cast`: command-line utilities for EVM contracts, transactions, and chain data
- `anvil`: local Ethereum development node
- `chisel`: Solidity REPL

The repository is a Cargo workspace. Core crates live under `crates/`, docs for
contributors live under `docs/dev/`, and Solidity fixtures and integration test
projects live under `testdata/`.

## Commands

```bash
cargo build --workspace                               # Build the workspace
cargo nextest run --workspace                         # Run tests
cargo +nightly fmt --all                              # Format Rust code
cargo +nightly fmt --all -- --check                   # Check Rust formatting
cargo +nightly clippy --workspace --all-targets --all-features # Lint Rust code
cargo deny check                                      # Check dependencies
cargo shear                                           # Check unused dependencies
```

Rust formatting uses nightly.

## Architecture

- `crates/forge`: Forge CLI and test/build workflows
- `crates/cast`: Cast CLI commands
- `crates/anvil`: local Ethereum node
- `crates/chisel`: Solidity REPL
- `crates/cheatcodes`: Forge cheatcode definitions and implementations
- `crates/common`: shared CLI, shell, compile, and terminal utilities
- `crates/config`: Foundry configuration
- `crates/debugger`: debugger support
- `crates/lint`: Solidity linter
- `crates/script`: script execution support
- `crates/verify`: contract verification support

Foundry's EVM execution tooling is built around `revm`. Cheatcodes are calls to
the fixed cheatcode address and are dispatched through the cheatcode inspector.
Custom network behavior for `anvil`, `forge`, and `cast` is implemented through
the EVM networks crate.

For symbolic execution work under `crates/evm/symbolic`, read
`crates/evm/symbolic/AGENTS.md` before editing.

## Testing

- Add tests for code changes that fix behavior or add functionality.
- Use focused unit tests for small pure logic.
- Use integration tests for CLI behavior and larger workflows.
- Tests that use forking must contain `fork` in their name.
- Forge integration fixtures live under `testdata/`.
- Lint rule tests live under `crates/lint/testdata/` with blessed `.stderr`
  output.

For CLI and integration tests:

- Put Forge CLI coverage under `crates/forge/tests/cli/` and Cast CLI coverage
  under `crates/cast/tests/cli/`.
- Use the existing `forgetest!`, `forgetest_init!`, and `casttest!` macros to
  create isolated test projects and command handles.
- Assert command output with snapbox helpers such as `assert_success()`,
  `assert_failure()`, `stdout_eq(str![...])`, `stderr_eq(str![...])`, and
  `assert_empty_stdout()`.
- For JSON output, use `assert_json_stdout(...)` or `assert_json_stderr(...)`
  so comparisons are parsed as JSON and unordered where appropriate.
- Prefer full output snapshots with redactions over ad hoc `String::contains`
  checks or manual `serde_json::Value` inspection.

For lint rules:

- Add a Solidity test file under `crates/lint/testdata/`.
- Use `//~WARN:` and `//~NOTE:` annotations for expected diagnostics.
- Regenerate blessed output with `cargo bless-lints`.
- Run lint UI tests with `cargo nextest run -p forge --test ui`.

For script work, keep the two execution phases separate: `ScriptArgs::execute`
runs the script, while on-chain simulation only executes the collected
broadcastable transactions. `--resume` resumes publishing transactions; it does
not recreate the original `--broadcast` state.

For fuzz or invariant corpus coverage work, `forge test --showmap-out <DIR>`
replays persisted corpus entries and writes AFL `showmap`-style coverage files.

## CLI Output

Foundry CLIs follow a stdout/stderr contract:

- stdout is the command's machine-readable primary result
- stderr is for warnings, errors, progress, status text, prompts, and banners
- `--json` changes stdout format, not channel cleanliness
- `--quiet` suppresses diagnostics and progress, not the command result
- verbosity flags such as `-vvv` must not change stdout content

Use the `sh_*` macros from `foundry_common::io`:

- `sh_println!` / `sh_print!`: primary stdout result only
- `sh_status!`: status prose on stderr
- `sh_progress!`: progress on stderr
- `sh_warn!`: recoverable warnings on stderr
- `sh_err!`: errors on stderr
- `prompt!`: prompt on stderr and read from stdin

Do not use `println!`, `print!`, `eprintln!`, or `eprint!`; workspace clippy
configuration forbids them.

## Configuration

When adding or changing a `foundry.toml` setting:

1. Define the field and its documentation in `crates/config`, including an
   explicit default and any required serde behavior. Keep related settings in a
   dedicated nested config type when they form a coherent section.
2. Wire the setting through every command that consumes it. If a CLI flag can
   override the setting, resolve precedence in one shared place and test config,
   CLI, and combined behavior.
3. Add focused config parsing and serialization tests. Update the `forge config`
   and default-config snapshots when the serialized surface changes.
4. For renamed or moved settings, preserve compatibility when practical and add
   a targeted deprecation warning that points to the canonical key. Test aliases,
   profiles, inheritance, environment variables, collisions, and malformed values
   where those providers are affected.
5. Document the setting in `foundry-rs/book` under
   `src/pages/config/reference/`, including its section, type, default, environment
   variable when supported, behavior, and a valid TOML example. Update the config
   reference navigation and `default-config.mdx` in the same documentation PR.
6. Keep CLI option text in the Rust clap definition; the book's CLI reference is
   generated from command help and should not be edited by hand.

Use the implementation, defaults, and tests as the source of truth. Do not merge
new user-facing configuration without the corresponding book update.

## Cheatcodes

When adding a cheatcode:

1. Add the Solidity definition in `crates/cheatcodes/spec/src/vm.rs`.
2. Implement the generated call type in `crates/cheatcodes/`.
3. Update `spec::Cheatcodes::new` if `Vm` gained a struct, enum, error, or event.
4. Run `cargo cheats` twice to update generated JSON assets.
5. Add an integration test under `testdata/default/cheats/`.

Cheatcode functions and structs must be documented and function parameters must
be named.

## Commit and PR Style

Default format is conventional commits:

```text
type: description
type(scope): description
type(scope)!: breaking description
```

Use `feat`, `fix`, `perf`, `chore`, `docs`, `test`, or `refactor`. Check recent
`git log` output before committing to match the repository's current style.

- Use imperative mood.
- Keep the description under 50 characters when practical.
- Do not end the description with a period.
- Include a body for performance changes, bug fixes, and complex changes.
- For performance changes, include measurements.
- PR titles should follow the same format as commit messages.

PR descriptions should explain what changed and why in flowing prose. Link
related issues and PRs when they exist. Include only real measurements, and do
not include validation/testing boilerplate such as "Validated with", "Tested
with", or command lists unless explicitly requested. Do not use templates,
bullet lists, or long essays. When writing PR bodies from scripts, use a file or
heredoc with real newlines; never pass escaped `\n` sequences.

### Performance PRs

When drafting or updating a PR body for a performance-related change, benchmark
the feature branch against `master` or the user-specified base before writing the
performance claims.

- Use the local benchmark runners under `benches/` unless the user explicitly
  asks for GitHub Actions or the Derek/decofe automation.
- Use `foundry-bench` when the claim is about elapsed time for a Foundry command
  on an existing Solidity project: `forge build`, cached rebuilds, `forge test`,
  fuzz-test replay, isolated tests, coverage, or focused symbolic tests.
- For invariant or campaign-style benchmarking, use `foundry-scfuzzbench`; this
  is the local equivalent of the `derek bench invariant`/`decofe bench
  invariant` PR flow, which publishes a `scfuzzbench` event.
- The local runners do not compare two local refs in one invocation. Run the
  baseline and candidate separately, with identical benchmark inputs, timeout,
  worker count, environment, target repository, and output schema.
- For branch-vs-base PR comparisons, use the profiling profile
  (`FOUNDRY_BENCH_LOCAL_BUILD_PROFILE=profiling`) rather than an ad hoc debug or
  release build. Keep ordinary `foundry-bench --versions local` comparisons on
  the default release distribution profile.
- Include only benchmarks that exercise the changed path. Do not pad the PR body
  with unrelated benchmark suites.
- Report both wall-time results and domain counters when available, for example
  solver queries, reported solver time, throughput, coverage relscore/relcov,
  or invariant findings.
- If results are neutral, noisy, or regress a secondary metric, state that
  directly. Do not convert noise into a performance claim.
- Keep the PR body short: one paragraph explaining the optimization and why it
  is correct, followed by a `### Results` table.
- Exact benchmark commands and result-table mechanics in `benches/README.md`.

## Notes

- Use `RUST_LOG=<filter>` for debugging CLI internals, for example
  `RUST_LOG=forge` or `RUST_LOG=cast`.
- Disclose AI assistance in PRs when used, per `CONTRIBUTING.md`.
- Do not send spelling-only or grammar-only documentation PRs.
- Keep release feature lists aligned between the root `Makefile` and release
  workflows when changing published CLI feature surfaces.

## Code Style

- Comments end with periods (except URLs)
- Files end with LF and trailing newline
- Follow existing patterns
- Never expose secrets

### Rust

- Put doc comments before attributes, always: `/// ...` comes before `#[derive]`, `#[inline]`, `#[cfg]`, and every other attribute.
- Put module documentation at the top of the module file with inner doc comments (`//! ...`), not on the `mod` item in the parent module.
- NEVER put imports inside functions unless required for `#[cfg(...)]` gating. All imports go at the top of the file.
- Group all `use` imports together. Keep `pub use` imports in a separate group. For local module re-exports, write `mod x;` before `pub use x;`; for re-exporting another module or external crate, use `use x;`, then a blank line, then `pub use y;`, then a blank line before local `mod my_mod; pub use my_mod::*;`.
- In `Cargo.toml`, generally group optional dependencies for a feature together. Put a comment immediately above the group containing only the feature name, for example `# jit`.
- Prefer `let Some(x) = x else { return };` / `let Ok(x) = x else { return };` over `match x { Some(x) => x, _ => return }`.
- Use `let ... else` only for a single early-exit guard. When multiple conditions or patterns gate the same block, prefer a combined `if let` / `let` chain instead of several sequential `let ... else` statements.
- Use combined `if let` chains (`if let Some(x) = x && let Some(y) = y { ... }`) instead of nesting (`if let Some(x) = x { if let Some(y) = y { ... } }`).
- In loops, prefer an `if let` chain around the loop body over multiple `let ... else { continue };` statements when the body only runs if all patterns match.
- NEVER use `ref` / `ref mut` in patterns as the first resort. Always prefer borrowing the expression with `&` / `&mut` instead.
- Avoid specifying type hints in variables unless absolutely necessary (e.g. `HashMap<_, Vec<_>>` for `x.entry(y).or_default().push(z)` where type inference won't work). Rely on the compiler.
- When type hints are needed, prefer turbofish (`let x = Type::<X, Y>::new()`) over annotation (`let x: Type<X, Y> = Type::new()`).
