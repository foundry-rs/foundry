use foundry_test_utils::ui_runner;
use std::{env, path::Path};

const FORGE_CMD: &str = env!("CARGO_BIN_EXE_forge");
const FORGE_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn main() -> impl std::process::Termination {
    let forge_cmd = Path::new(FORGE_CMD);
    let forge_dir = Path::new(FORGE_DIR);
    let lint_testdata = forge_dir.parent().unwrap().join("lint").join("testdata");

    ui_runner::run_tests("lint", forge_cmd, &lint_testdata)
}
