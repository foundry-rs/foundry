//! Contains various tests for checking forge's commands
use ethers::solc::{
    artifacts::{BytecodeHash, Metadata},
    ConfigurableContractArtifact,
};
use foundry_cli_test_utils::{
    ethers_solc::PathStyle,
    forgetest, forgetest_ignore, forgetest_init, next_http_rpc_endpoint,
    util::{pretty_err, read_string, TestCommand, TestProject},
};
use foundry_config::{parse_with_profile, BasicConfig, Chain, Config, SolidityErrorCode};
use std::fs;
use yansi::Paint;

// import forge utils as mod
#[allow(unused)]
#[path = "../../src/utils.rs"]
mod forge_utils;

// tests `--help` is printed to std out
forgetest!(print_help, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("--help");
    cmd.assert_non_empty_stdout();
});

// checks that `clean` can be invoked even if out and cache don't exist
forgetest!(can_clean_non_existing, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// checks that `cache clean` can be invoked and cleans the foundry cache
// this test is not isolated and modifies ~ so it is ignored
forgetest_ignore!(can_cache_clean, |_: TestProject, mut cmd: TestCommand| {
    let cache_dir = Config::foundry_cache_dir().unwrap();
    let path = cache_dir.as_path();
    fs::create_dir_all(path).unwrap();
    cmd.args(["cache", "clean"]);
    cmd.assert_empty_stdout();

    assert!(!path.exists());
});

// checks that `cache clean <chain>` can be invoked and cleans the chain cache
// this test is not isolated and modifies ~ so it is ignored
forgetest_ignore!(can_cache_clean_chain, |_: TestProject, mut cmd: TestCommand| {
    let cache_dir =
        Config::foundry_chain_cache_dir(Chain::Named(ethers::prelude::Chain::Mainnet)).unwrap();
    let path = cache_dir.as_path();
    fs::create_dir_all(path).unwrap();
    cmd.args(["cache", "clean", "mainnet"]);
    cmd.assert_empty_stdout();

    assert!(!path.exists());
});

// checks that `cache clean <chain> --blocks 100,101` can be invoked and cleans the chain block
// caches this test is not isolated and modifies ~ so it is ignored
forgetest_ignore!(can_cache_clean_blocks, |_: TestProject, mut cmd: TestCommand| {
    let chain = Chain::Named(ethers::prelude::Chain::Mainnet);
    let block1 = 100;
    let block2 = 102;
    let block1_cache_dir = Config::foundry_block_cache_dir(chain, block1).unwrap();
    let block2_cache_dir = Config::foundry_block_cache_dir(chain, block2).unwrap();
    let block1_path = block1_cache_dir.as_path();
    let block2_path = block2_cache_dir.as_path();
    fs::create_dir_all(block1_path).unwrap();
    fs::create_dir_all(block2_path).unwrap();
    cmd.args(["cache", "clean", "mainnet", "--blocks", "100,101"]);
    cmd.assert_empty_stdout();

    assert!(!block1_path.exists());
    assert!(!block2_path.exists());
});

// checks that init works
forgetest!(can_init_repo_with_config, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    let foundry_toml = prj.root().join(Config::FILE_NAME);
    assert!(!foundry_toml.exists());

    cmd.args(["init", "--force"]).arg(prj.root());
    cmd.assert_non_empty_stdout();

    let file = Config::find_config_file().unwrap();
    assert_eq!(foundry_toml, file);

    let s = read_string(&file);
    let _config: BasicConfig = parse_with_profile(&s).unwrap().unwrap().1;

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
    assert!(prj.root().join("lib/forge-std").exists());
    assert!(!prj.root().join("lib/forge-std/.git").exists());
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
    assert!(prj.root().join("lib/forge-std").exists());
});

// Checks that remappings.txt and .vscode/settings.json is generated
forgetest!(can_init_vscode, |prj: TestProject, mut cmd: TestCommand| {
    prj.wipe();

    cmd.arg("init").arg(prj.root()).arg("--vscode");
    cmd.assert_non_empty_stdout();

    let settings = prj.root().join(".vscode/settings.json");
    assert!(settings.is_file());
    let settings: serde_json::Value = ethers::solc::utils::read_json_file(&settings).unwrap();
    assert_eq!(
        settings,
        serde_json::json!({
             "solidity.packageDefaultDependenciesContractsDirectory": "src",
            "solidity.packageDefaultDependenciesDirectory": "lib"
        })
    );

    let remappings = prj.root().join("remappings.txt");
    assert!(remappings.is_file());
    let content = std::fs::read_to_string(remappings).unwrap();
    assert_eq!(content, "ds-test/=lib/forge-std/lib/ds-test/src/\nforge-std/=lib/forge-std/src/");
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

// checks that `clean` also works with the "out" value set in Config
forgetest_init!(can_clean_config, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    let config = Config { out: "custom-out".into(), ..Default::default() };
    prj.write_config(config);
    cmd.arg("build");
    cmd.assert_non_empty_stdout();

    // default test contract is written in custom out directory
    let artifact = prj.root().join("custom-out/Contract.t.sol/ContractTest.json");
    assert!(artifact.exists());

    cmd.forge_fuse().arg("clean");
    cmd.output();
    assert!(!artifact.exists());
});

// checks that extra output works
forgetest_init!(can_emit_extra_output, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    cmd.args(["build", "--extra-output", "metadata"]);
    cmd.assert_non_empty_stdout();

    let artifact_path = prj.paths().artifacts.join("Contract.sol/Contract.json");
    let artifact: ConfigurableContractArtifact =
        ethers::solc::utils::read_json_file(artifact_path).unwrap();
    assert!(artifact.metadata.is_some());

    cmd.forge_fuse().args(["build", "--extra-output-files", "metadata", "--force"]).root_arg();
    cmd.assert_non_empty_stdout();

    let metadata_path = prj.paths().artifacts.join("Contract.sol/Contract.metadata.json");
    let _artifact: Metadata = ethers::solc::utils::read_json_file(metadata_path).unwrap();
});

// checks that extra output works
forgetest_init!(can_emit_multiple_extra_output, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
    cmd.args(["build", "--extra-output", "metadata", "ir-optimized", "--extra-output", "ir"]);
    cmd.assert_non_empty_stdout();

    let artifact_path = prj.paths().artifacts.join("Contract.sol/Contract.json");
    let artifact: ConfigurableContractArtifact =
        ethers::solc::utils::read_json_file(artifact_path).unwrap();
    assert!(artifact.metadata.is_some());
    assert!(artifact.ir.is_some());
    assert!(artifact.ir_optimized.is_some());

    cmd.forge_fuse()
        .args([
            "build",
            "--extra-output-files",
            "metadata",
            "ir-optimized",
            "evm.bytecode.sourceMap",
            "--force",
        ])
        .root_arg();
    cmd.assert_non_empty_stdout();

    let metadata_path = prj.paths().artifacts.join("Contract.sol/Contract.metadata.json");
    let _artifact: Metadata = ethers::solc::utils::read_json_file(metadata_path).unwrap();

    let iropt = prj.paths().artifacts.join("Contract.sol/Contract.iropt");
    std::fs::read_to_string(iropt).unwrap();

    let sourcemap = prj.paths().artifacts.join("Contract.sol/Contract.sourcemap");
    std::fs::read_to_string(sourcemap).unwrap();
});

forgetest!(can_print_warnings, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >0.8.9;
contract Greeter {
    function foo(uint256 a) public {
        uint256 x = 1;
    }
}
   "#,
        )
        .unwrap();

    // explicitly set to run with 0.8.10
    let config = Config { solc: Some("0.8.10".into()), ..Default::default() };
    prj.write_config(config);

    cmd.arg("build");

    let output = cmd.stdout_lossy();
    assert!(output.contains(
        "
Compiler run successful (with warnings)
warning[5667]: Warning: Unused function parameter. Remove or comment out the variable name to silence this warning.
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

    assert!(cmd.stdout_lossy().ends_with(
        "
Compiler run successful
"
    ));
});

// Tests that the `run` command works correctly
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
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 1751
== Return ==
== Logs ==
  script ran
",
        Paint::green("Script ran successfully.")
    ),));
});

// Tests that the run command can run arbitrary functions
forgetest!(can_execute_run_command_with_sig, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function myFunction() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("run").arg(script).arg("--sig").arg("myFunction()");
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 1751
== Return ==
== Logs ==
  script ran
",
        Paint::green("Script ran successfully.")
    ),));
});

// Tests that the run command can run functions with arguments
forgetest!(can_execute_run_command_with_args, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    event log_uint(uint);
    function run(uint256 a, uint256 b) external {
        emit log_string("script ran");
        emit log_uint(a);
        emit log_uint(b);
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("run").arg(script).arg("--sig").arg("run(uint256,uint256)").arg("1").arg("2");
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 3957
== Return ==
== Logs ==
  script ran
  1
  2
",
        Paint::green("Script ran successfully.")
    ),));
});

// Tests that the run command can run functions with return values
forgetest!(can_execute_run_command_with_returned, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function run() external returns (uint256 result, uint8) {
        emit log_string("script ran");
        return (255, 3);
    }
}"#,
        )
        .unwrap();
    cmd.arg("run").arg(script);
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 1836
== Return ==
result: uint256 255
1: uint8 3
== Logs ==
  script ran
",
        Paint::green("Script ran successfully.")
    )));
});

// tests that the `inspect` command works correctly
forgetest!(can_execute_inspect_command, |prj: TestProject, mut cmd: TestCommand| {
    // explicitly set to include the ipfs bytecode hash
    let config = Config { bytecode_hash: BytecodeHash::Ipfs, ..Default::default() };
    prj.write_config(config);
    let contract_name = "Foo";
    let _ = prj
        .inner()
        .add_source(
            contract_name,
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Foo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
    "#,
        )
        .unwrap();

    // Remove the ipfs hash from the metadata
    let mut dynamic_bytecode = "0x608060405234801561001057600080fd5b5060c08061001f6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063c040622614602d575b600080fd5b60336035565b005b7f0b2e13ff20ac7b474198655583edf70dedd2c1dc980e329c4fbb2fc0748b796b6040516080906020808252600a908201526939b1b934b83a103930b760b11b604082015260600190565b60405180910390a156fea264697066735822122065c066d19101ad1707272b9a884891af8ab0cf5a0e0bba70c4650594492c14be64736f6c634300080a0033\n".to_string();
    let ipfs_start = dynamic_bytecode.len() - (24 + 64);
    let ipfs_end = ipfs_start + 65;
    dynamic_bytecode.replace_range(ipfs_start..ipfs_end, "");
    cmd.arg("inspect").arg(contract_name).arg("bytecode");
    let mut output = cmd.stdout_lossy();
    output.replace_range(ipfs_start..ipfs_end, "");

    // Compare the static bytecode
    assert_eq!(dynamic_bytecode, output);
});

// test that `forge snapshot` commands work
forgetest!(can_check_snapshot, |prj: TestProject, mut cmd: TestCommand| {
    prj.insert_ds_test();

    prj.inner()
        .add_source(
            "ATest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract ATest is DSTest {
    function testExample() public {
        assertTrue(true);
    }
}
   "#,
        )
        .unwrap();
    prj.inner()
        .add_source(
            "BTest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract BTest is DSTest {
    function testExample() public {
        assertTrue(true);
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("snapshot");

    let out = cmd.stdout();

    assert!(
        out.contains(&format!(
            "Running 1 test for {}/src/BTest.t.sol:BTest",
            prj.root().to_string_lossy()
        )) && out.contains(&format!(
            "Running 1 test for {}/src/ATest.t.sol:ATest",
            prj.root().to_string_lossy()
        ))
    );

    cmd.arg("--check");
    let _ = cmd.output();
});

// test that `forge build` does not print `(with warnings)` if there arent any
forgetest!(can_compile_without_warnings, |prj: TestProject, mut cmd: TestCommand| {
    let config = Config {
        ignored_error_codes: vec![SolidityErrorCode::SpdxLicenseNotProvided],
        ..Default::default()
    };
    prj.write_config(config);
    prj.inner()
        .add_source(
            "A",
            r#"
pragma solidity 0.8.10;
contract A {
    function testExample() public {}
}
   "#,
        )
        .unwrap();

    cmd.args(["build", "--force"]);
    let out = cmd.stdout();
    // no warnings
    assert!(out.trim().contains("Compiler run successful"));
    assert!(!out.trim().contains("Compiler run successful (with warnings)"));

    // don't ignore errors
    let config = Config { ignored_error_codes: vec![], ..Default::default() };
    prj.write_config(config);
    let out = cmd.stdout();

    assert!(out.trim().contains("Compiler run successful (with warnings)"));
    assert!(
      out.contains(
                    r#"Warning: SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information."#
        )
    );
});

// test against a local checkout, useful to debug with local ethers-rs patch
forgetest_ignore!(can_compile_local_spells, |_: TestProject, mut cmd: TestCommand| {
    let current_dir = std::env::current_dir().unwrap();
    let root = current_dir
        .join("../../foundry-integration-tests/testdata/spells-mainnet")
        .to_string_lossy()
        .to_string();
    println!("project root: \"{root}\"");

    let eth_rpc_url = next_http_rpc_endpoint();
    let dss_exec_lib = "src/DssSpell.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4";

    cmd.args([
        "test",
        "--root",
        root.as_str(),
        "--fork-url",
        eth_rpc_url.as_str(),
        "--fork-block-number",
        "14435000",
        "--libraries",
        dss_exec_lib,
        "-vvv",
    ]);
    cmd.print_output();
});

// test that a failing `forge build` does not impact followup builds
forgetest!(can_build_after_failure, |prj: TestProject, mut cmd: TestCommand| {
    prj.insert_ds_test();

    prj.inner()
        .add_source(
            "ATest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract ATest is DSTest {
    function testExample() public {
        assertTrue(true);
    }
}
   "#,
        )
        .unwrap();
    prj.inner()
        .add_source(
            "BTest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract BTest is DSTest {
    function testExample() public {
        assertTrue(true);
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("build");
    cmd.assert_non_empty_stdout();
    prj.assert_cache_exists();
    prj.assert_artifacts_dir_exists();

    let syntax_err = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract CTest is DSTest {
    function testExample() public {
        THIS WILL CAUSE AN ERROR
    }
}
   "#;

    // introduce contract with syntax error
    prj.inner().add_source("CTest.t.sol", syntax_err).unwrap();

    // `forge build --force` which should fail
    cmd.arg("--force");
    cmd.assert_err();

    // but ensure this cleaned cache and artifacts
    assert!(!prj.paths().artifacts.exists());
    assert!(!prj.cache_path().exists());

    // still errors
    cmd.forge_fuse().arg("build");
    cmd.assert_err();

    // resolve the error by replacing the file
    prj.inner()
        .add_source(
            "CTest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract CTest is DSTest {
    function testExample() public {
         assertTrue(true);
    }
}
   "#,
        )
        .unwrap();

    cmd.assert_non_empty_stdout();
    prj.assert_cache_exists();
    prj.assert_artifacts_dir_exists();

    // ensure cache is unchanged after error
    let cache = fs::read_to_string(prj.cache_path()).unwrap();

    // introduce the error again but building without force
    prj.inner().add_source("CTest.t.sol", syntax_err).unwrap();
    cmd.assert_err();

    // ensure unchanged cache file
    let cache_after = fs::read_to_string(prj.cache_path()).unwrap();
    assert_eq!(cache, cache_after);
});
