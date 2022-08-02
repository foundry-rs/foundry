use std::path::PathBuf;

#[derive(Debug)]
pub struct DocConfig {
    pub templates: Option<PathBuf>,
    pub output: Option<PathBuf>,
}
