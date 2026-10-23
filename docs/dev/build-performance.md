# Build performance

Contributor settings that shorten local build and test cycles. They are per-developer choices, not
repository defaults.

## Feature unification

Cargo resolves features separately for each package selection. Switching between
`cargo build --workspace`, `cargo build -p forge`, `cargo build -p cast` and `cargo test -p forge`
therefore compiles some dependencies with different features, and rebuilds them and everything that
depends on them.

With nightly Cargo, `feature-unification = "workspace"` makes every selection use the feature set
of the whole workspace, so switching between selections reuses existing artifacts. Each
single-package build then compiles more: for example, `cargo build -p cast` grows from 978 to 1046
units.

Enable it for every Cargo invocation of the current user in `~/.cargo/config.toml`, or in a local
`.cargo/config.toml` that you do not commit:

```toml
[unstable]
feature-unification = true

[resolver]
feature-unification = "workspace"
```

Or enable it for a single command:

```sh
cargo build -p cast -Zfeature-unification --config 'resolver.feature-unification="workspace"'
```

See the Cargo documentation for [`feature-unification`][feature-unification].

[feature-unification]: https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#feature-unification
