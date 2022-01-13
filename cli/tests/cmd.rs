//! Contains various tests for checking forge's commands
use foundry_cli_test_utils::{
    ethers_solc::PathStyle,
    forgetest, pretty_eq,
    util::{pretty_err, read_string, TestCommand, TestProject},
};
use foundry_config::{parse_with_profile, BasicConfig, Config};
use std::{env::set_current_dir, fs};

// tests `--help` is printed to std out
forgetest!(print_help, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("--help");
    cmd.assert_non_empty_stdout();
});

// tests config gets printed to std out
forgetest!(can_show_config, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("config");
    let expected = Config::load().to_string_pretty().unwrap().trim().to_string();
    pretty_eq!(expected, cmd.stdout().trim().to_string());
});

// checks that `clean` can be invoked even if out and cache don't exist
forgetest!(can_clean_non_existing, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// checks that init works
forgetest!(can_init_repo_with_config, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("init").arg(prj.root());
    println!("{}", String::from_utf8_lossy(&cmd.output().stdout) );
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(foundry_toml.exists());

    set_current_dir(prj.root()).unwrap();
    let file = Config::find_config_file().unwrap();
    assert_eq!(foundry_toml, file);

    let s = read_string(&file);
    println!("{}", s.clone());
    let basic: BasicConfig = parse_with_profile(&s).unwrap().unwrap().1;
    assert_eq!(basic, Config::load().into_basic());
});

// checks that init works repeatedly
forgetest!(can_init_repeatedly, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("init").arg(prj.root());
    cmd.assert_non_empty_stdout();
    let foundry_toml = prj.root().join(Config::FILE_NAME);

    for _ in 0..3 {
        assert!(foundry_toml.exists());
        pretty_err(&foundry_toml, fs::remove_file(&foundry_toml));
        cmd.assert_non_empty_stdout();
    }
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
