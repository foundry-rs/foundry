//! Contains various tests for checking forge's commands
use foundry_cli_test_utils::{
    ethers_solc::PathStyle,
    forgetest, pretty_eq,
    util::{TestCommand, TestProject},
};
use foundry_config::Config;

// tests `--help` is printed to std out
forgetest!(print_help, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("--help");
    cmd.assert_non_empty_stdout();
});

// tests config gets printed to std out
forgetest!(can_show_config, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("config");
    let expected = Config::from(Config::figment()).to_string_pretty().unwrap().trim().to_string();
    pretty_eq!(expected, cmd.stdout().trim().to_string());
});

// checks that `clean` can be invoked even if out and cache don't exist
forgetest!(can_clean_non_existing, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// checks that `clean` removes dapptools style paths
forgetest!(can_clean, |prj: TestProject, mut cmd: TestCommand| {
    prj.assert_create_dirs_exists();
    prj.assert_style_paths_exist(PathStyle::Dapptools);
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// checks that `clean` removes hardhat style paths
forgetest!(can_clean_hardhat, PathStyle::HardHat, |prj: TestProject, mut cmd: TestCommand| {
    prj.assert_create_dirs_exists();
    prj.assert_style_paths_exist(PathStyle::HardHat);
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});
