# foundry-binder

Utilities for generating bindings for solidity projects in one step.

First add `foundry-binder` to your cargo build-dependencies.

```toml
[build-dependencies]
foundry-binder = { git = "https://github.com/foundry-rs/foundry" }
# required in order to enable ssh support in [libgit2](https://github.com/rust-lang/git2-rs)
git2 = "0.16.1"
```

```rust
use foundry_binder::{Binder, RepositoryBuilder, Url};

// github repository url
const REPO_URL: &str = "<the-url-of-the-project>";

// the release tag for which to generate bindings for
const RELEASE_TAG: &str = "v3.0.0";

/// This clones the project, builds the project and generates rust bindings
fn generate() {
    let binder =
        Binder::new(RepositoryBuilder::new(Url::parse(REPO_URL).unwrap())
            // generate bindings for this release tag
            // if not set, then the default branch will be used
            .tag(RELEASE_TAG))
            // keep build artifacts in `artifacts` folder
            .keep_artifacts("artifacts");

    binder.generate().expect("Failed to generate bindings")
}

fn main() {
    // only generate if `FRESH_BINDINGS` env var is set
    if std::env::var("FRESH_BINDINGS").is_ok() {
        generate()
    }
}
```
