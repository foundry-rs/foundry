#![cfg(test)]

use crate::analyzer::Analyzer;
use foundry_cli::opts::BuildOpts;
use foundry_config::Config;
use std::fs;
use tempfile::{TempDir, tempdir};
use tower_lsp::lsp_types::Url;

/// Creates a temporary project, initializes the analyzer with a linter,
/// runs the analysis, and returns the necessary components for a test.
pub fn setup_analyzer(contracts: &[(&str, &str)]) -> (Url, Analyzer, TempDir) {
    let temp_dir = tempdir().expect("Failed to create temporary directory");
    let root = temp_dir.path();

    let src_dir = root.join("src");
    fs::create_dir(&src_dir).expect("Failed to create src directory");

    for (name, content) in contracts {
        fs::write(src_dir.join(name), content).expect("Failed to write contract to temporary file");
    }

    // Manually construct the Config to ensure paths are set correctly for the test
    let mut config = Config::default();
    config.root = root.into();
    config.src = src_dir.clone().into();
    config.cache_path = root.join("cache").into();
    config.out = root.join("out").into();

    let file_to_test = contracts[0].0;
    let file_path = src_dir.join(file_to_test);
    let file_uri = dunce::canonicalize(&file_path)
        .ok()
        .and_then(|path| Url::from_file_path(path).ok())
        .expect("Failed to create file URI");

    let mut analyzer = Analyzer::new(config, BuildOpts::default());
    analyzer.analyze().expect("Failed to analyze project");

    (file_uri, analyzer, temp_dir)
}
