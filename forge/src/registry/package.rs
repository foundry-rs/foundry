use std::path::PathBuf;
use std::rc::Rc;
use foundry_config::manifest::Manifest;

/// Represents a single dependency
#[derive(Clone)]
pub struct Package {
    inner: Rc<PackageInner>,
}

#[derive(Clone)]
struct PackageInner {
    /// The manifest of package if it contains a `foundry.toml`
    manifest: Option<Manifest>,
    /// Where this package is stored
    path: PathBuf,
    /// Whether this dependency has additional git submodules
    has_submodules: bool
}
