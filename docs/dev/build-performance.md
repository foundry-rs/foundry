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

Enable it in `~/.cargo/config.toml`, or in a `.cargo/config.toml` in a directory above your checkout,
which Cargo merges with the repository's own `.cargo/config.toml`:

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

## Parallel compiler frontend

With nightly rustc, `-Zthreads` runs the compiler frontend on several threads. On a 24-core,
48-thread EPYC 9254, a clean dev-profile `cargo build --workspace` took 101.5 s and 101.3 s without
the flag and 81.2 s and 81.8 s with `-Zthreads=8`. That is about 20% less wall time for about 23%
more CPU time.

Enable it in `~/.cargo/config.toml`:

```toml
[build]
rustflags = ["-Zthreads=8"]
```

Or set it for a single command:

```sh
RUSTFLAGS="-Zthreads=8" cargo build --workspace
```

Cargo ignores `build.rustflags` when `RUSTFLAGS` or `target.<triple>.rustflags` is set. Changing the
flags rebuilds everything.

See the rustc documentation for [`-Zthreads`][threads].

## Shared generics

rustc shares generic instantiations between crates only at opt-level 0, 1, `s` and `z`. The
`[profile.dev.package]` overrides in the root `Cargo.toml` set opt-level 3 for crates such as
`serde`, `serde_json`, `alloy`, `revm`, `solar` and `foundry-compilers`, so every downstream crate
instantiates their generics again. `-Zshare-generics=y` turns sharing back on for all crates.

On the same machine, a clean `cargo build --workspace` took 101.7 s and 102.6 s without the flag and
98.6 s and 99.8 s with it. That is about 3% less wall time and 12% less CPU time. Incremental
rebuilds after editing a function in `foundry-common` or `foundry-cheatcodes` and the `forge test`
runtime on `testdata` did not change.

Enable it in `~/.cargo/config.toml`. It combines with `-Zthreads` in one list:

```toml
[build]
rustflags = ["-Zshare-generics=y"]
```

Or set it for a single command:

```sh
RUSTFLAGS="-Zshare-generics=y" cargo build --workspace
```

The same caveats about `RUSTFLAGS`, `target.<triple>.rustflags` and rebuilds apply.

See the rustc documentation for [`-Zshare-generics`][share-generics].

[threads]: https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/threads.html
[share-generics]: https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/share-generics.html
