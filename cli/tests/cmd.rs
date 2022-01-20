//! Contains various tests for checking forge's commands
use foundry_cli_test_utils::{
    ethers_solc::{remappings::Remapping, PathStyle},
    forgetest, forgetest_ignore, forgetest_init, pretty_eq,
    util::{pretty_err, read_string, TestCommand, TestProject},
};
use foundry_config::{parse_with_profile, BasicConfig, Config};
use pretty_assertions::assert_eq;
use std::{
    env::{self},
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
forgetest!(can_show_config, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("config");
    cmd.set_current_dir(prj.root());
    let expected = Config::load().to_string_pretty().unwrap().trim().to_string();
    assert_eq!(expected, cmd.stdout().trim().to_string());
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

    cmd.set_current_dir(prj.root());
    let file = Config::find_config_file().unwrap();
    assert_eq!(foundry_toml, file);

    let s = read_string(&file);
    let basic: BasicConfig = parse_with_profile(&s).unwrap().unwrap().1;
    // check ds-test is detected
    assert_eq!(
        basic.remappings,
        vec![Remapping::from_str("ds-test/=lib/ds-test/src/").unwrap().into()]
    );
    assert_eq!(basic, Config::load().into_basic());

    // can detect root
    assert_eq!(prj.root(), forge_utils::find_project_root_path().unwrap());
    let nested = prj.root().join("nested/nested");
    pretty_err(&nested, std::fs::create_dir_all(&nested));

    // even if nested
    cmd.set_current_dir(&nested);
    assert_eq!(prj.root(), forge_utils::find_project_root_path().unwrap());
});

// checks that init works repeatedly
forgetest!(can_init_repo_repeatedly, |prj: TestProject, mut cmd: TestCommand| {
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(!foundry_toml.exists());

    cmd.arg("init").arg(prj.root());
    cmd.assert_non_empty_stdout();

    for _ in 0..2 {
        assert!(foundry_toml.exists());
        pretty_err(&foundry_toml, fs::remove_file(&foundry_toml));
        cmd.assert_non_empty_stdout();
    }
});

// checks that config works
// - foundry.toml is properly generated
// - paths are resolved properly
// - config supports overrides from env, and cli
forgetest_init!(can_override_config, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());

    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(foundry_toml.exists());
    let file = Config::find_config_file().unwrap();
    assert_eq!(foundry_toml, file);

    let config = forge_utils::load_config();
    let profile = Config::load();
    assert_eq!(config, profile.clone().sanitized());

    // ensure remappings contain test
    assert_eq!(profile.remappings.len(), 1);
    assert_eq!("ds-test/=lib/ds-test/src/".to_string(), profile.remappings[0].to_string());
    // the loaded config has resolved, absolute paths
    assert_eq!(
        format!("ds-test/={}/", prj.root().join("lib/ds-test/src").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    cmd.arg("config");
    let expected = profile.to_string_pretty().unwrap();
    pretty_eq!(expected.trim().to_string(), cmd.stdout().trim().to_string());

    // remappings work
    let remappings_txt = prj.create_file("remappings.txt", "from-file/=lib/from-file/");
    let config = forge_utils::load_config();
    assert_eq!(
        format!("from-file/={}/", prj.root().join("lib/from-file").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    // env vars work
    cmd.set_env("DAPP_REMAPPINGS", "other/=lib/other/src/");
    let config = forge_utils::load_config();
    assert_eq!(
        format!("other/={}/", prj.root().join("lib/other/src").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    let config = prj.config_from_output(["--remappings", "from-cli/=lib-from-cli"]);
    assert_eq!(
        format!("from-cli/={}/", prj.root().join("lib-from-cli").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    cmd.unset_env("DAPP_REMAPPINGS");
    pretty_err(&remappings_txt, fs::remove_file(&remappings_txt));

    cmd.set_cmd(prj.bin()).args(["config", "--basic"]);
    let expected = profile.into_basic().to_string_pretty().unwrap();
    pretty_eq!(expected.trim().to_string(), cmd.stdout().trim().to_string());
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
