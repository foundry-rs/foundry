//! Contains various tests for checking forge's commands
use foundry_cli_test_utils::{
    ethers_solc::{remappings::Remapping, PathStyle},
    forgetest, forgetest_ignore, pretty_eq,
    util::{pretty_err, read_string, TestCommand, TestProject},
};
use foundry_config::{parse_with_profile, BasicConfig, Config};
use std::{
    env::{self, set_current_dir},
    fs,
    str::FromStr,
};

// import forge utils as mod
#[allow(unused)]
#[path = "../src/utils.rs"]
mod forge_utils;

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
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(!foundry_toml.exists());

    cmd.arg("init").arg(prj.root());
    cmd.assert_non_empty_stdout();

    set_current_dir(prj.root()).unwrap();
    let file = Config::find_config_file().unwrap();
    assert_eq!(foundry_toml, file);

    let s = read_string(&file);
    let basic: BasicConfig = parse_with_profile(&s).unwrap().unwrap().1;
    // check ds-test is detected
    assert_eq!(
        basic.remappings,
        vec![Remapping::from_str("ds-test/=lib/ds-test/src").unwrap().into()]
    );
    assert_eq!(basic, Config::load().into_basic());

    // can detect root
    assert_eq!(prj.root(), forge_utils::find_project_root_path().unwrap());
    let nested = prj.root().join("nested/nested");
    pretty_err(&nested, std::fs::create_dir_all(&nested));

    // even if nested
    set_current_dir(&nested).unwrap();
    assert_eq!(prj.root(), forge_utils::find_project_root_path().unwrap());
});

// checks that init works repeatedly
forgetest!(can_init_repo_repeatedly, |prj: TestProject, mut cmd: TestCommand| {
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(!foundry_toml.exists());

    cmd.arg("init").arg(prj.root());
    cmd.assert_non_empty_stdout();

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

// test against a local checkout, useful to debug with local ethers-rs patch
forgetest_ignore!(can_compile_local_spells, |_: TestProject, mut cmd: TestCommand| {
    let current_dir = std::env::current_dir().unwrap();
    let root = current_dir
        .join("../../foundry-integration-tests/testdata/spells-mainnet")
        .to_string_lossy()
        .to_string();
    println!("project root: \"{}\"", root);

    let eth_rpc_url = env::var("ETH_RPC_URL").unwrap();
    let dss_exec_lib = "src/DssSpell.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4";

    cmd.args([
        "build",
        "--root",
        root.as_str(),
        "--fork-url",
        eth_rpc_url.as_str(),
        "--libraries",
        dss_exec_lib,
        "-vvv",
        "--force",
    ]);
    cmd.print_output();
});
