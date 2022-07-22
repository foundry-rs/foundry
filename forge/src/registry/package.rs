use foundry_config::manifest::Manifest;
use std::{path::PathBuf, rc::Rc};

/// Represents a single dependency
#[derive(Clone)]
pub struct Package {
    inner: Rc<PackageInner>,
}

// === impl Package ===

impl Package {
    /// Creates a new `Package` with the given path and manifest
    pub fn new(manifest: Option<Manifest>, path: impl into<PathBuf>) -> Package {
        Package {
            inner: Rc::new(PackageInner {
                manifest,
                path: path.into(),
                // TODO
                has_submodules: false,
            }),
        }
    }
}

#[derive(Clone)]
struct PackageInner {
    /// The manifest of package if it contains a `foundry.toml`
    manifest: Option<Manifest>,
    /// Where this package is stored
    path: PathBuf,
    /// Whether this dependency has additional git submodules
    has_submodules: bool,
}
