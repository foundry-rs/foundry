//! Contains various tests for checking the `forge create` subcommand

use crate::{
    constants::*,
    utils::{self, EnvExternalities},
};
use anvil::{spawn, NodeConfig};
use ethers::{
    solc::{artifacts::BytecodeHash, remappings::Remapping},
    types::Address,
};
use foundry_config::Config;
use foundry_test_utils::{
    forgetest, forgetest_async,
    util::{OutputExt, TestCommand, TestProject},
};
use std::{path::PathBuf, str::FromStr};

/// This will insert _dummy_ contract that uses a library
///
/// **NOTE** This is intended to be linked against a random address and won't actually work. The
/// purpose of this is _only_ to make sure we can deploy contracts linked against addresses.
///
/// This will create a library `remapping/MyLib.sol:MyLib`
///
/// returns the contract argument for the create command
fn setup_with_simple_remapping(prj: &TestProject) -> String {
    // explicitly set remapping and libraries
    let config = Config {
        remappings: vec![Remapping::from_str("remapping/=lib/remapping/").unwrap().into()],
        libraries: vec![format!("remapping/MyLib.sol:MyLib:{:?}", Address::random())],
        ..Default::default()
    };
    prj.write_config(config);

    prj.inner()
        .add_source(
            "LinkTest",
            r#"
// SPDX-License-Identifier: MIT
import "remapping/MyLib.sol";
contract LinkTest {
    function foo() public returns (uint256) {
        return MyLib.foobar(1);
    }
}
"#,
        )
        .unwrap();

    prj.inner()
        .add_lib(
            "remapping/MyLib",
            r#"
// SPDX-License-Identifier: MIT
library MyLib {
    function foobar(uint256 a) public view returns (uint256) {
    	return a * 100;
    }
}
"#,
        )
        .unwrap();

    "src/LinkTest.sol:LinkTest".to_string()
}

fn setup_oracle(prj: &TestProject) -> String {
    let config = Config {
        libraries: vec![format!(
            "./src/libraries/ChainlinkTWAP.sol:ChainlinkTWAP:{:?}",
            Address::random()
        )],
        ..Default::default()
    };
    prj.write_config(config);

    prj.inner()
        .add_source(
            "Contract",
            r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;
import {ChainlinkTWAP} from "./libraries/ChainlinkTWAP.sol";
contract Contract {
    function getPrice() public view returns (int latest) {
        latest = ChainlinkTWAP.getLatestPrice(0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE);
    }
}
"#,
        )
        .unwrap();

    prj.inner()
        .add_source(
            "libraries/ChainlinkTWAP",
            r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library ChainlinkTWAP {
   function getLatestPrice(address base) public view returns (int256) {
        return 0;
   }
}
"#,
        )
        .unwrap();

    "src/Contract.sol:Contract".to_string()
}

/// configures the `TestProject` with the given closure and calls the `forge create` command
fn create_on_chain<F>(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand, f: F)
where
    F: FnOnce(&TestProject) -> String,
{
    if let Some(info) = info {
        let contract_path = f(&prj);
        cmd.arg("create");
        cmd.args(info.create_args()).arg(contract_path);

        let out = cmd.stdout_lossy();
        let _address = utils::parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {out}"));
    }
}

// tests `forge` create on goerli if correct env vars are set
forgetest!(can_create_simple_on_goerli, |prj: TestProject, cmd: TestCommand| {
    create_on_chain(EnvExternalities::goerli(), prj, cmd, setup_with_simple_remapping);
});

// tests `forge` create on goerli if correct env vars are set
forgetest!(can_create_oracle_on_goerli, |prj: TestProject, cmd: TestCommand| {
    create_on_chain(EnvExternalities::goerli(), prj, cmd, setup_oracle);
});

// tests `forge` create on mumbai if correct env vars are set
forgetest!(can_create_oracle_on_mumbai, |prj: TestProject, cmd: TestCommand| {
    create_on_chain(EnvExternalities::mumbai(), prj, cmd, setup_oracle);
});

// tests that we can deploy the template contract
forgetest_async!(
    #[serial_test::serial]
    can_create_template_contract,
    |prj: TestProject, mut cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let rpc = handle.http_endpoint();
        let wallet = handle.dev_wallets().next().unwrap();
        let pk = hex::encode(wallet.signer().to_bytes());
        cmd.args(["init", "--force"]);
        cmd.assert_non_empty_stdout();

        // explicitly byte code hash for consistent checks
        let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
        prj.write_config(config);

        cmd.forge_fuse().args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
            "--use",
            "solc:0.8.15",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
        ]);

        cmd.unchecked_output().stdout_matches_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/can_create_template_contract.stdout"),
        );

        cmd.unchecked_output().stdout_matches_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/can_create_template_contract-2nd.stdout"),
        );
    }
);

// tests that we can deploy the template contract
forgetest_async!(
    #[serial_test::serial]
    can_create_using_unlocked,
    |prj: TestProject, mut cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let rpc = handle.http_endpoint();
        let dev = handle.dev_accounts().next().unwrap();
        cmd.args(["init", "--force"]);
        cmd.assert_non_empty_stdout();

        // explicitly byte code hash for consistent checks
        let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
        prj.write_config(config);

        cmd.forge_fuse().args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
            "--use",
            "solc:0.8.15",
            "--rpc-url",
            rpc.as_str(),
            "--from",
            format!("{dev:?}").as_str(),
            "--unlocked",
        ]);

        cmd.unchecked_output().stdout_matches_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/can_create_using_unlocked.stdout"),
        );

        cmd.unchecked_output().stdout_matches_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/can_create_using_unlocked-2nd.stdout"),
        );
    }
);
