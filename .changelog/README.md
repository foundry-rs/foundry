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

Release automation accepts only canonical `X.Y.Z` versions and `vX.Y.Z` tags, plus
strict release candidates in the form `X.Y.Z-rcN` and `vX.Y.Z-rcN` where `N >= 1`.
Legacy prerelease forms are excluded from transition state. Manual dispatches must
confirm their exact source and target tags: `start` turns the checked-in stable candidate
into `rc1`, `advance` moves the latest same-core RC to the next RC, and `promote` removes
the RC suffix. Starting and advancing require and consume changelog fragments; promotion
requires zero fragments and leaves `CHANGELOG.md` unchanged. Source RC tags must exist,
be the latest same-core RC, form a contiguous sequence from `rc1`, and be ancestors of
the release commit.

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

## Pull requests

Pull requests must add or update a `.changelog/*.md` entry unless a maintainer applies
the existing `L-ignore` label. Use the exemption for changes that should not appear in
release notes, such as CI-only or repository-maintenance changes.

Each entry maps one or more workspace package names to `patch`, `minor`, or `major` and
includes a non-empty release note:

```md
---
forge: minor
cast: patch
---

Added a Forge feature and fixed the related Cast behavior.
```

PR validation is deterministic and independent of the advisory AI suggestion. It rejects
malformed frontmatter, unknown packages, invalid bump values, empty package mappings,
empty notes, and entries that are only deleted.
