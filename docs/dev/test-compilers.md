# Test compilers

Ordinary tests use `SOLC_VERSION` (0.8.35) and version-switching tests use
`OTHER_SOLC_VERSION` (0.8.26), defined in
[`crates/test-utils/src/util.rs`](../../crates/test-utils/src/util.rs).
Use these constants when adding Rust tests. Solidity fixtures that check artifact
versions must keep their pragmas and version-qualified artifact names in sync.

`testdata/foundry.toml` caps auto-detection at the shared default, so installing a
newer compiler for a compatibility test does not change the fixture compiler.
The CLI `testdata_compiler_versions` test checks the resolved compiler inventory.
Its repeated 0.8.35 entries represent separate compiler profiles, not extra downloads.

## Required exceptions

| Compiler | Coverage |
| --- | --- |
| 0.5.17 | Legacy Solidity test execution in `test_cmd/mod.rs` |
| 0.6.12 | Maker fork fixtures under `testdata/default/fork/` |
| 0.7.6 | Legacy fixtures, lint rejection, and Chisel EVM-version normalization |
| 0.8.4 | `coverage --ir-minimum` before the inliner compiler setting existed |
| Latest supported (currently 0.8.37) | SVM release checks and Amsterdam execution in `svm.rs` |

External-project tests compile third-party repositories with their own constraints.
Fork, verification, and explorer tests may also compile downloaded source with its
original compiler. The shared pair is not a limit on these compatibility tests.

## Auditing compiler downloads

A pragma inventory includes parser, formatter, linter, and resolver fixtures that
never invoke solc. For example, `compiler.rs` resolves 0.8.4, 0.8.11, and 0.8.33
without installing them. Config parsing tests and fake compiler version strings
also do not imply a download. Broad pragmas such as `^0.8.18` allow the shared
default and do not need mechanical rewrites.

Gather all tracked Solidity pragmas, including inline Rust fixtures:

```sh
git grep -n 'pragma solidity' -- '*.sol' '*.rs'
git grep -h -o 'pragma solidity[^;]*;' -- '*.sol' '*.rs' | sort | uniq -c | sort -nr
```

Inspect explicit overrides and install calls as well:

```sh
rg -n 'find_or_install|blocking_install|Solc::install|config\.solc|--use' crates
(cd testdata && forge compiler resolve --json)
```

The local fixture project resolves 0.6.12, 0.8.26, and 0.8.35. CI caches `~/.svm`
separately from compiled fixtures. CI creates this directory before tests so SVM
uses it instead of a platform-specific data directory. The cache is keyed by OS,
architecture, SVM platform, and matrix entry. Each run can save newly required versions. The final CI step lists
installed binaries even when tests fail; restored binaries appear in this list too.
When measuring cold downloads, use an empty compiler cache rather than counting
all version strings in source or treating the restored inventory as new downloads.
