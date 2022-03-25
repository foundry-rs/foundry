//! Contains various tests for checking forge commands related to config values
use forge::executor::opts::EvmOpts;
use foundry_cli_test_utils::{
    ethers_solc::remappings::Remapping,
    forgetest, forgetest_init, pretty_eq,
    util::{pretty_err, TestCommand, TestProject},
};
use foundry_config::{Config, OptimizerDetails};
use pretty_assertions::assert_eq;
use std::fs;

// import forge utils as mod
#[allow(unused)]
#[path = "../src/utils.rs"]
mod forge_utils;

// tests config gets printed to std out
forgetest!(can_show_config, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("config");
    cmd.set_current_dir(prj.root());
    let expected = Config::load().to_string_pretty().unwrap().trim().to_string();
    assert_eq!(expected, cmd.stdout().trim().to_string());
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
    let figment = Config::figment_with_root(prj.root()).merge(("debug", false));
    let evm_opts: EvmOpts = figment.extract().unwrap();
    assert_eq!(evm_opts.fork_url, Some(url.to_string()));
});

// checks that we can set various config values
forgetest_init!(can_set_config_values, |prj: TestProject, _cmd: TestCommand| {
    let config = prj.config_from_output(["--via-ir"]);
    assert!(config.via_ir);
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
    let config = Config { solc: Some("0.8.10".into()), ..Default::default() };
    prj.write_config(config);

    cmd.arg("build");

    assert!(cmd.stdout_lossy().ends_with(
        "
Compiler run successful
",
    ));
});

// tests that `--use <solc>` works
forgetest!(can_use_solc, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.10;
contract Foo {}
   "#,
        )
        .unwrap();

    cmd.args(["build", "--use", "0.8.11"]);

    let stdout = cmd.stdout_lossy();
    assert!(stdout.contains("Compiler run successful"));

    cmd.fuse().args(["build", "--force", "--use", "solc:0.8.11"]).root_arg();

    assert!(stdout.contains("Compiler run successful"));

    // fails to use solc that does not exist
    cmd.fuse().args(["build", "--use", "this/solc/does/not/exist"]);
    assert!(cmd.stderr_lossy().contains("this/solc/does/not/exist does not exist"));

    // 0.8.11 was installed in previous step, so we can use the path to this directly
    let local_solc = ethers::solc::Solc::find_svm_installed_version("0.8.11").unwrap().unwrap();
    cmd.fuse().args(["build", "--force", "--use"]).arg(local_solc.solc).root_arg();
    assert!(stdout.contains("Compiler run successful"));
});

// test to ensure yul optimizer can be set as intended
forgetest!(can_set_yul_optimizer, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Foo {
    function bar() public pure {
       assembly {
            let result_start := msize()
       }
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("build");

    assert!(
        cmd.stderr_lossy().contains(
"The msize instruction cannot be used when the Yul optimizer is activated because it can change its semantics. Either disable the Yul optimizer or do not use the instruction."
        )
    );

    // disable yul optimizer explicitly
    let config = Config {
        optimizer_details: Some(OptimizerDetails { yul: Some(false), ..Default::default() }),
        ..Default::default()
    };
    prj.write_config(config);

    assert!(cmd.stdout_lossy().ends_with(
        "
Compiler run successful
",
    ));
});

forgetest_init!(can_parse_dapp_libraries, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    cmd.set_env(
        "DAPP_LIBRARIES",
        "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6",
    );
    let config = cmd.config();
    assert_eq!(
        config.libraries,
        vec!["src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6".to_string(),]
    );
});
