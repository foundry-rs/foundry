# Developer documentation

These documents describe contributor workflows and invariants that span multiple Foundry crates.
They are not a second user manual or a manually maintained map of every workspace dependency.

## Documentation ownership

Keep each fact in the source that owns it and link to that source elsewhere:

| Content | Canonical location |
| --- | --- |
| User-facing guides, configuration, and CLI workflows | [Foundry Book][foundry-book] |
| Lint reference explanations | [`crates/lint/docs/`](../../crates/lint/docs/README.md), imported by the Book |
| Crate and module APIs, invariants, and implementation details | Source Rustdoc, published as [Foundry Rustdoc][foundry-rustdoc] |
| Cross-crate contributor workflows | `docs/dev/` or [`CONTRIBUTING.md`](../../CONTRIBUTING.md) |
| Agent-only repository instructions | [`AGENTS.md`](../../AGENTS.md) |
| Release-facing changes | [Changelog fragments](../../.changelog/README.md) |

Do not copy generated CLI reference text or crate dependency lists into `docs/dev`. Update CLI help
or Rustdoc at the source, then link to the generated documentation.

## Setup and validation

Install [Rust][rust], Make, and [cargo-nextest][nextest]. Foundry uses the stable toolchain for
normal builds and the latest nightly toolchain for formatting and Clippy.

```sh
make build
make test
make pr
```

Use focused unit tests for local logic and integration tests for user-visible workflows. Tests that
use forking must contain `fork` in their name. Forge and Cast CLI tests live under
`crates/forge/tests/cli/` and `crates/cast/tests/cli/`; shared integration fixtures live in
`crates/test-utils`, and Solidity fixtures live under `testdata/`.

## Maintained guides

- [Cheatcodes](./cheatcodes.md) explains cheatcode generation, dispatch, and implementation.
- [Debugging](./debugging.md) collects contributor debugging techniques.
- [Lint rules](./lintrules.md) covers the lint registry, UI fixtures, and documentation contract.
- [Custom EVM integrations](./networks.md) describes network selection, execution ownership,
  state lifecycles, tool dispatch, and CI coverage.
- [Output channels](./output-channels.md) defines the stdout/stderr contract for Foundry commands.
- [Scripting](./scripting.md) documents the internal script execution and broadcast pipeline.
- [Showmap corpus replay](./showmap.md) documents the persisted-corpus coverage workflow and file
  format.

## Updating documentation

Update documentation at the canonical location in the ownership table alongside implementation
changes. For lint reference pages, update [`crates/lint/docs/`](../../crates/lint/docs/README.md)
in the Foundry PR; the Book's
weekly update generates the published pages. Keep CLI help in the command definitions and crate or
module contracts in Rustdoc next to the implementation.
Add or update a guide here only when contributors need a cross-crate workflow or invariant that does
not have a single source owner.

Every maintained guide must be linked from this index. Prefer links to canonical documentation over
duplicated instructions so updates cannot drift independently.

## CI and release features

CI runs tests through cargo-nextest. Nightly and stable release builds derive their enabled
functionality from `RUST_FEATURES` in `.github/workflows/release.yml` and
`.github/workflows/docker-publish.yml`. Keep those lists aligned with the default `FEATURES` in the
root `Makefile` so published binaries expose the same surface as local release builds.

For contribution policy and support channels, see [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

[foundry-book]: https://getfoundry.sh
[foundry-rustdoc]: https://foundry-rs.github.io/foundry/
[nextest]: https://nexte.st/docs/installation/pre-built-binaries/#with-cargo-binstall
[rust]: https://rustup.rs/

## Automated dependency pull requests

The Tempo, Solar, and forge-std update workflows use a repository-scoped GitHub App token to
push their branches and create or update pull requests. GitHub requires manual approval for
PR workflow runs triggered with `GITHUB_TOKEN`; using an App token lets the normal PR checks
start automatically. See [GitHub's token documentation][github-token].

Before enabling these workflows, install the automation App on `foundry-rs/foundry` with
**Contents: read and write** and **Pull requests: read and write** repository permissions.
Configure the repository Actions variable `FOUNDRY_RELEASE_APP_ID` with its App ID and the
Actions secret `FOUNDRY_RELEASE_APP_PRIVATE_KEY` with its private key. These are the same
credentials used by release automation. Each dependency job requests a short-lived token
restricted to this repository and those two permissions; its built-in `GITHUB_TOKEN` is read-only.
Missing or invalid App credentials fail the update instead of silently creating PRs whose CI
needs approval. No personal access token or individual maintainer's account is required.

After setup, dispatch a dependency update workflow when an upstream update is available and
confirm that its PR checks start without approval. Check both a newly created PR and a later
update to its branch. These workflows only create or update PRs; merging remains a separate step.

[github-token]: https://docs.github.com/en/actions/concepts/security/github_token
