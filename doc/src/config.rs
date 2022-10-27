use std::path::{Path, PathBuf};

// TODO: move & merge w/ Figment
/// The doc builder configuration
#[derive(Debug)]
pub struct DocConfig {
    /// The project root
    pub root: PathBuf,
    /// Path to Solidity source files.
    pub sources: PathBuf,
    /// Output path.
    pub out: PathBuf,
    /// The documentation title.
    pub title: String,
}

impl DocConfig {
    /// Construct new documentation
    pub fn new(root: &Path) -> Self {
        DocConfig { root: root.to_owned(), ..Default::default() }
    }

    /// Get the output directory
    pub fn out_dir(&self) -> PathBuf {
        self.root.join(&self.out)
    }
}

impl Default for DocConfig {
    fn default() -> Self {
        DocConfig {
            root: PathBuf::new(),
            sources: PathBuf::new(),
            out: PathBuf::from("docs"),
            title: "".to_owned(),
        }
    }
}
