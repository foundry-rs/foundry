use foundry_test_utils::runner;
use std::path::Path;

const FORGE_CMD: &'static str = env!("CARGO_BIN_EXE_forge");
const FORGE_DIR: &'static str = env!("CARGO_MANIFEST_DIR");

#[test]
fn forge_lint_ui_tests() -> eyre::Result<()> {
    let forge_cmd = Path::new(FORGE_CMD);
    let forge_dir = Path::new(FORGE_DIR);
    let lint_testdata = forge_dir.parent().unwrap().join("lint").join("testdata");

    runner::run_tests("lint", &forge_cmd, &lint_testdata, true)
}
