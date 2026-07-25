# Changelogs

Foundry uses [`tempoxyz/changelogs`](https://github.com/tempoxyz/changelogs) pinned to
`74f07b39c7c85773575e32472dfd5298ec6baaf4`:

```sh
cargo install \
  --git https://github.com/tempoxyz/changelogs \
  --rev 74f07b39c7c85773575e32472dfd5298ec6baaf4 \
  --locked
```

All CLI commands and GitHub Action references must use that exact revision. Do not use
the check action's built-in AI installer, which does not pin the installed CLI.

## Version contract

- The workspace manifest records the next release candidate.
- If the manifest is already ahead of the latest stable tag, release preparation uses that
  candidate without applying another bump. At adoption, `1.7.2` follows `v1.7.1` and maps
  directly to `v1.7.2`.
- Nightlies use the candidate version, keeping `1.7.2-nightly` SemVer-newer than stable
  `1.7.1`. The candidate must advance when stable advances.
- A stable `vX.Y.Z` tag must point to a commit whose workspace version is exactly `X.Y.Z`.
- Generated `CHANGELOG.md` history starts at adoption. Historical releases are not
  backfilled.

Root changelog format gives every discovered package one unified version. A read-only dry
run at the pinned revision discovers 37 release-planned packages;
`forge-sol-macro-gen` and `foundry-test-utils` are excluded because they set
`publish = false`, while all 39 workspace packages inherit the root version.

Foundry does not publish its workspace crates to a registry. Automation must not invoke
`changelogs publish` or the root `tempoxyz/changelogs` composite action. Foundry-owned
automation owns version pull requests and tags.

Run the pinned CLI locally with:

```sh
changelogs doctor
changelogs status
changelogs version --dry-run
```
