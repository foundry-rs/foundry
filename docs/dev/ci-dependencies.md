# CI dependency boundary

```text
checkout SHA → cooldown → Socket fetch → offline vendor → source artifact
                                                              ↓
                               verify checkout + hash → parallel frozen builds
```

[`dependencies.yml`](../../.github/workflows/dependencies.yml) checks cooldown and
runs `sfw cargo fetch --locked` in an empty Cargo home. Cargo uses the Git CLI for
Socket's proxy certificate support. `cargo vendor --frozen` then packages only
those fetched sources without executing package build scripts. Failed approval
prevents publication; source caches cannot bypass the current policy.

[`setup-build`](../../.github/actions/setup-build/action.yml) downloads the exact
artifact ID supplied by its caller, verifies its SHA256 and checkout identity,
and configures a fresh offline Cargo home. Builds use `--frozen`, source replacement
and compiler-only sccache; they cannot resolve a different Cargo graph. Tests and
docs reuse their caller's bundle. No cross-run source or target cache is restored.

[`solc-releases.mjs`](../../.github/scripts/solc-releases.mjs) bundles commit-pinned
compiler metadata so `svm-rs-builds` need not fetch release lists during compilation.
It mirrors svm-rs 0.5.27's platform rules: review it when updating that dependency
or the solc snapshot. These JSON inputs are not packages scanned by Socket.

This draft covers regular Cargo CI, Tempo/MPP, flaky/deploy tests and crate checks,
not every supply-chain input. Before rollout, merge the
[secure-runner prerequisite](https://github.com/tempoxyz/gh-actions/pull/149),
validate fork/Dependabot OIDC and the full OS matrix, and measure cold/warm CI time.
Release/Docker/benchmark builders, the external cargo-deny workflow, Python/Node/Bun
and generated binding graphs, bootstrap tools and OS packages need separate coverage.
Python installs use Socket but are not part of this Cargo bundle.

`--frozen` restricts Cargo, not arbitrary build-script or test networking. Runner
egress enforcement and separation of publishing secrets remain necessary;
secure-runner's audit fallback is not a network sandbox. Protect the workflow and
policy through repository review/rulesets. No scanner guarantees malware-free code.
