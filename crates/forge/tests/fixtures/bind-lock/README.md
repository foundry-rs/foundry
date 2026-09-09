# Bind test dependency lock

This fixture pins the dependency graph used when Forge's integration tests compile generated Rust
bindings. Its manifest contains the union of dependencies emitted by `forge bind`. Tests preserve
the generated manifest and prune only unused direct dependencies from the copied root lock entry.

Seed updates from the workspace lock so all compatible dependencies retain their already-vetted
versions, then let Cargo resolve the Alloy 1.x-specific portion:

```sh
cp Cargo.lock crates/forge/tests/fixtures/bind-lock/Cargo.lock
cargo check --manifest-path crates/forge/tests/fixtures/bind-lock/Cargo.toml
```

Keep the fixture's package name and version aligned with the `forge bind` defaults because Cargo
records the root package in the lockfile.
