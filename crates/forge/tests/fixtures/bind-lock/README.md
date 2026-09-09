# Bind test dependency lock

This fixture pins the dependency graph used when Forge's integration tests compile generated Rust
bindings. Its manifest contains the union of dependencies emitted by `forge bind`; tests add the
unused `serde_with` dependency when necessary so every generated crate can share this lockfile.

Update the lockfile explicitly when the generated manifest changes:

```sh
cargo update --manifest-path crates/forge/tests/fixtures/bind-lock/Cargo.toml
```

Keep the fixture's package name and version aligned with the `forge bind` defaults because Cargo
records the root package in the lockfile.
