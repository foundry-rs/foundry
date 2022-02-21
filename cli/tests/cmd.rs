//! Contains various tests for checking forge's commands
use ansi_term::Colour;
use ethers::solc::{artifacts::Metadata, ConfigurableContractArtifact};
use evm_adapters::evm_opts::{EvmOpts, EvmType};
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

    cmd.args(["init", "--force"]).arg(prj.root());
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
    assert_eq!(basic, Config::load_with_root(prj.root()).into_basic());

    // can detect root
    assert_eq!(prj.root(), forge_utils::find_project_root_path().unwrap());
    let nested = prj.root().join("nested/nested");
    pretty_err(&nested, std::fs::create_dir_all(&nested));

    // even if nested
    cmd.set_current_dir(&nested);
    assert_eq!(prj.root(), forge_utils::find_project_root_path().unwrap());
});

// checks that init works repeatedly
forgetest!(can_init_repo_repeatedly_with_force, |prj: TestProject, mut cmd: TestCommand| {
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(!foundry_toml.exists());

    prj.wipe();

    cmd.arg("init").arg(prj.root());
    cmd.assert_non_empty_stdout();

    cmd.arg("--force");

    for _ in 0..2 {
        assert!(foundry_toml.exists());
        pretty_err(&foundry_toml, fs::remove_file(&foundry_toml));
        cmd.assert_non_empty_stdout();
    }
});

// Checks that a forge project can be initialized without creating a git repository
forgetest!(can_init_no_git, |prj: TestProject, mut cmd: TestCommand| {
    prj.wipe();

    cmd.arg("init").arg(prj.root()).arg("--no-git");
    cmd.assert_non_empty_stdout();
    prj.assert_config_exists();

    assert!(!prj.root().join(".git").exists());
    assert!(prj.root().join("lib/ds-test").exists());
    assert!(!prj.root().join("lib/ds-test/.git").exists());
});

// Checks that quiet mode does not print anything
forgetest!(can_init_quiet, |prj: TestProject, mut cmd: TestCommand| {
    prj.wipe();

    cmd.arg("init").arg(prj.root()).arg("-q");
    let _ = cmd.output();
});

// `forge init` does only work on non-empty dirs
forgetest!(can_init_non_empty, |prj: TestProject, mut cmd: TestCommand| {
    prj.create_file("README.md", "non-empty dir");
    cmd.arg("init").arg(prj.root());
    cmd.assert_err();

    cmd.arg("--force");
    cmd.assert_non_empty_stdout();
    assert!(prj.root().join(".git").exists());
    assert!(prj.root().join("lib/ds-test").exists());
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
    let profile = Config::load_with_root(prj.root());
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
    assert_eq!(expected.trim().to_string(), cmd.stdout().trim().to_string());

    // remappings work
    let remappings_txt = prj.create_file("remappings.txt", "ds-test/=lib/ds-test/from-file/");
    let config = forge_utils::load_config();
    assert_eq!(
        format!("ds-test/={}/", prj.root().join("lib/ds-test/from-file").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    // env vars work
    cmd.set_env("DAPP_REMAPPINGS", "ds-test/=lib/ds-test/from-env/");
    let config = forge_utils::load_config();
    assert_eq!(
        format!("ds-test/={}/", prj.root().join("lib/ds-test/from-env").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    let config = prj.config_from_output(["--remappings", "ds-test/=lib/ds-test/from-cli"]);
    assert_eq!(
        format!("ds-test/={}/", prj.root().join("lib/ds-test/from-cli").display()),
        Remapping::from(config.remappings[0].clone()).to_string()
    );

    let config = prj.config_from_output(["--remappings", "other-key/=lib/other/"]);
    assert_eq!(config.remappings.len(), 2);
    assert_eq!(
        format!("other-key/={}/", prj.root().join("lib/other").display()),
        Remapping::from(config.remappings[1].clone()).to_string()
    );

    cmd.unset_env("DAPP_REMAPPINGS");
    pretty_err(&remappings_txt, fs::remove_file(&remappings_txt));

    cmd.set_cmd(prj.bin()).args(["config", "--basic"]);
    let expected = profile.into_basic().to_string_pretty().unwrap();
    pretty_eq!(expected.trim().to_string(), cmd.stdout().trim().to_string());
});

forgetest_init!(can_detect_config_vals, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    let url = "http://127.0.0.1:8545";
    let config = prj.config_from_output(["--no-auto-detect", "--rpc-url", url]);
    assert!(!config.auto_detect_solc);
    assert_eq!(config.eth_rpc_url, Some(url.to_string()));

    let mut config = Config::load_with_root(prj.root());
    config.eth_rpc_url = Some("http://127.0.0.1:8545".to_string());
    config.auto_detect_solc = false;
    // write to `foundry.toml`
    prj.create_file(
        Config::FILE_NAME,
        &config.to_string_pretty().unwrap().replace("eth_rpc_url", "eth-rpc-url"),
    );
    let config = prj.config_from_output(["--force"]);
    assert!(!config.auto_detect_solc);
    assert_eq!(config.eth_rpc_url, Some(url.to_string()));
});

// checks that `clean` removes dapptools style paths
forgetest_init!(can_get_evm_opts, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    let url = "http://127.0.0.1:8545";
    let config = prj.config_from_output(["--rpc-url", url, "--ffi"]);
    assert_eq!(config.eth_rpc_url, Some(url.to_string()));
    assert!(config.ffi);

    cmd.set_env("FOUNDRY_ETH_RPC_URL", url);
    let figment = Config::figment_with_root(prj.root())
        .merge(("evm_type", EvmType::Sputnik))
        .merge(("debug", false));
    let evm_opts: EvmOpts = figment.extract().unwrap();
    assert_eq!(evm_opts.fork_url, Some(url.to_string()));
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

// checks that extra output works
forgetest_init!(can_emit_extra_output, |prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["build", "--extra-output", "metadata"]);
    cmd.assert_non_empty_stdout();

    let artifact_path = prj.paths().artifacts.join("Contract.sol/Contract.json");
    let artifact: ConfigurableContractArtifact =
        ethers::solc::utils::read_json_file(artifact_path).unwrap();
    assert!(artifact.metadata.is_some());

    cmd.fuse()
        .args(["build", "--extra-output-files", "metadata", "--force", "--root"])
        .arg(prj.root());
    cmd.assert_non_empty_stdout();

    let metadata_path = prj.paths().artifacts.join("Contract.sol/Contract.metadata.json");
    let _artifact: Metadata = ethers::solc::utils::read_json_file(metadata_path).unwrap();
});

forgetest!(can_set_solc_explicitly, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >0.8.9;
contract Greeter {}
   "#,
        )
        .unwrap();

    // explicitly set to run with 0.8.10
    let config = Config { solc_version: Some("0.8.10".parse().unwrap()), ..Default::default() };
    prj.write_config(config);

    cmd.arg("build");

    assert!(cmd.stdout_lossy().ends_with(
        "compiling...
Compiling 1 files with 0.8.10
Compilation finished successfully
Compiler run successful
success.
",
    ));
});

// tests that direct import paths are handled correctly
forgetest!(can_handle_direct_imports_into_src, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import {FooLib} from "src/FooLib.sol";
struct Bar {
    uint8 x;
}
contract Foo {
    mapping(uint256 => Bar) bars;
    function checker(uint256 id) external {
        Bar memory b = bars[id];
        FooLib.check(b);
    }
    function checker2() external {
        FooLib.check2(this);
    }
}
   "#,
        )
        .unwrap();

    prj.inner()
        .add_source(
            "FooLib",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import {Foo, Bar} from "src/Foo.sol";
library FooLib {
    function check(Bar memory b) public {}
    function check2(Foo f) public {}
}
   "#,
        )
        .unwrap();

    cmd.arg("build");

    assert_eq!(
        "compiling...
Compiling 2 files with 0.8.10
Compilation finished successfully
Compiler run successful
success.
",
        cmd.stdout_lossy()
    );
});

// tests that the `run` command works correctly
forgetest!(can_execute_run_command, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("run").arg(script);
    let output = cmd.stdout_lossy();
    assert_eq!(
        format!(
            "compiling...
Compiling 1 files with 0.8.10
Compilation finished successfully
success.
{}
Gas Used: 1751
== Logs ==
script ran
",
            Colour::Green.paint("Script ran successfully.")
        ),
        output
    );
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
