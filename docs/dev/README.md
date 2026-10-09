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
- [Editor integrations](../../editors/README.md) covers the VS Code Development Host,
  independent client builds, local packaging and Zed installation.
- [External compiler adapters](./external-compiler-adapters.md) defines the executable protocol,
  cache contract, and artifact integration for compiler-native EVM projects.
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

Maintainers select stable and release-candidate versions, update the workspace version and
`Cargo.lock` on the corresponding `release-X.Y.Z` or `release-X.Y.Z-rcN` branch, then dispatch the
[tag release workflow](../../.github/workflows/tag-release.yml) from `master` with the chosen `version`
and exact 40-character lowercase `commit` SHA. It checks the selected checkout, workspace and member
versions, and `cargo metadata --locked`. It reuses the latest full CI run for that SHA, or dispatches
`ci.yml` on the release branch if no eligible run exists. The branch must still point to the requested
SHA when CI is dispatched. PR CI is not eligible because it uses a reduced platform matrix. Failed,
canceled, missing, or unexpectedly skipped required checks block tagging; rerun the latest failed CI run explicitly.

After approval in `release-tag`, the workflow repeats validation and checks the latest CI attempt
before creating the immutable `vX.Y.Z[-rcN]` tag at the selected SHA. Existing tags are accepted only
when they resolve to that SHA, including annotated tags. Stable versions must exceed existing stable
tags; newer RC tags do not block stable maintenance releases. RC versions must exceed all release tags.
The coordinator then dispatches the [release workflow](../../.github/workflows/release.yml) on the tag.
That workflow requires the same full CI evidence before building, signing, attesting, and generating
PR-based notes in a draft GitHub release. The separate run preserves the tag ref in signing identities
and provenance. After reviewing the notes and successful build, run the
[finalization workflow](../../.github/workflows/finalize-release.yml) from `master` with that exact
tag. It verifies the release workflow and recorded Docker digest before publishing and promoting
eligible Docker aliases. Nightlies continue through the scheduled release workflow.

To retry a tagged build, rerun its release workflow run, or dispatch `release.yml` on the same tag with
`expected_commit` set to the full tested SHA. A successful tag workflow only confirms the build was
dispatched; finalization requires the tag's release workflow to succeed. Both workflow files must be
present on the default branch before dispatching them.

Before enabling this flow, repository administrators must install a dedicated GitHub App on this
repository with only Contents write permission. Configure the `release-tag` environment to allow
only `master`, require maintainer approval, and prevent self-review. Store its App ID as environment
variable `RELEASE_APP_ID` and its private key as environment secret `RELEASE_APP_PRIVATE_KEY`.
The workflow mints a short-lived token scoped to this repository only after validation and approval;
the ordinary workflow token handles CI/build dispatch and cannot create routine release tags.

Keep the existing tag update/deletion restrictions without a bypass for this App. Add a separate
active tag creation ruleset for `refs/tags/v*.*.*`, restricting creation to the App as its sole bypass
actor. Separate rulesets let the App create tags without allowing it to move or delete them. Nightly
tags remain outside the creation restriction. For an emergency, an administrator may temporarily
grant a designated maintainer bypass of the creation-only ruleset, verify the version, locked metadata,
and full CI at the exact SHA, create the tag, then remove that bypass. Never relax update/deletion
restrictions or move an existing release tag. Direct tagged builds still require successful full CI.

For contribution policy and support channels, see [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

[foundry-book]: https://getfoundry.sh
[foundry-rustdoc]: https://foundry-rs.github.io/foundry/
[nextest]: https://nexte.st/docs/installation/pre-built-binaries/#with-cargo-binstall
[rust]: https://rustup.rs/
