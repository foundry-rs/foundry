# Developer Docs

The Foundry project is organized as a regular [Cargo workspace][cargo-workspace].

## Installation requirements

- [Rust](https://rustup.rs/)
- Make

We use `cargo-nextest` as test runner (both locally and in the [CI](#ci)):

- [Nextest](https://nexte.st/docs/installation/pre-built-binaries/#with-cargo-binstall)

## Recommended

If you are working in VSCode, we recommend you install the [rust-analyzer](https://rust-analyzer.github.io/) extension, and use the following VSCode user settings:

```json
"editor.formatOnSave": true,
"rust-analyzer.rustfmt.extraArgs": ["+nightly"],
"[rust]": {
  "editor.defaultFormatter": "rust-lang.rust-analyzer"
}
```

Note that we use Rust's latest `nightly` for formatting. If you see `;` being inserted by your code editor it is a good indication you are on `stable`.

## Getting started

Build the project.

```sh
$ make build
```

Run all tests.

```sh
$ make test
```

Run all tests and linters in preparation for a PR.

```sh
$ make pr
```

## Contents

- [Architecture](./architecture.md)
- [Cheatcodes](./cheatcodes.md)
- [Debugging](./debugging.md)
- [Scripting](./scripting.md)

_Note: This is incomplete and possibly outdated_

## Getting in Touch

See also [Getting Help](../../README.md#getting-help)

## Issue Labels

Whenever a ticket is initially opened a [`T-needs-triage`](https://github.com/foundry-rs/foundry/issues?q=is%3Aissue+is%3Aopen+label%3AT-needs-triage) label is assigned. This means that a member has yet to correctly label it.

If this is your first time contributing have a look at our [`first-issue`](https://github.com/foundry-rs/foundry/issues?q=is%3Aissue+is%3Aopen+label%3A%22first+issue%22) tickets. These are tickets we think are a good way to get familiar with the codebase.

We classify the tickets in two major categories: [`T-feature`](https://github.com/foundry-rs/foundry/issues?q=is%3Aissue+is%3Aopen+label%3AT-feature) and [`T-bug`](https://github.com/foundry-rs/foundry/issues?q=is%3Aissue+is%3Aopen+label%3AT-bug). Additional labels are usually applied to help categorize the ticket for future reference.

We also make use of [`T-meta`](https://github.com/foundry-rs/foundry/issues?q=is%3Aissue+is%3Aopen+label%3AT-meta) aggregation tickets. These tickets are tickets to collect related features and bugs.

We also have [`T-discuss`](https://github.com/foundry-rs/foundry/issues?q=is%3Aissue+is%3Aopen+label%3AT-to-discuss) tickets that require further discussion before proceeding on an implementation. Feel free to jump into the conversation!

## CI

We use GitHub Actions for continuous integration (CI).

We use [cargo-nextest][nextest] as the test runner.

If `make test` passes locally, that's a good sign that CI will be green as well.

[foundry-book]: https://book.getfoundry.sh
[cargo-workspace]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[nextest]: https://nexte.st/
