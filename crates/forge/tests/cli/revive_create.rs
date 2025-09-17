//! Contains various tests for checking the `forge create --resolc` subcommand

use crate::utils::{network_private_key, network_rpc_key};
use alloy_primitives::Address;
use foundry_compilers::artifacts::remappings::Remapping;
use foundry_test_utils::{
    TestCommand, forgetest_serial, revive::PolkadotNode, snapbox::IntoData, util::TestProject,
};
use serial_test::serial;
use std::str::FromStr;

const CREATE_RESPONSE_PATTERN: &str = r#"[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!
Deployer: [..]
Deployed to: [..]
[TX_HASH]
"#;

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
    prj.update_config(|config| {
        config.remappings = vec![Remapping::from_str("remapping/=lib/remapping/").unwrap().into()];
        config.libraries = vec![format!("remapping/MyLib.sol:MyLib:{:?}", Address::random())];
    });

    prj.add_source(
        "LinkTest",
        r#"
import "remapping/MyLib.sol";
contract LinkTest {
    function foo() public pure returns (uint256) {
        return MyLib.foobar(1);
    }
}
"#,
    )
    .unwrap();

    prj.add_lib(
        "remapping/MyLib",
        r"
library MyLib {
    function foobar(uint256 a) public pure returns (uint256) {
    	return a * 100;
    }
}
",
    )
    .unwrap();

    "src/LinkTest.sol:LinkTest".to_string()
}

fn setup_oracle(prj: &TestProject) -> String {
    prj.update_config(|c| {
        c.libraries = vec![format!(
            "./src/libraries/ChainlinkTWAP.sol:ChainlinkTWAP:{:?}",
            Address::random()
        )];
    });

    prj.add_source(
        "Contract",
        r#"
import {ChainlinkTWAP} from "./libraries/ChainlinkTWAP.sol";
contract Contract {
    function getPrice() public pure returns (int latest) {
        latest = ChainlinkTWAP.getLatestPrice(0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE);
    }
}
"#,
    )
    .unwrap();

    prj.add_source(
        "libraries/ChainlinkTWAP",
        r"
library ChainlinkTWAP {
   function getLatestPrice(address) public pure returns (int256) {
        return 0;
   }
}
",
    )
    .unwrap();

    "src/Contract.sol:Contract".to_string()
}

fn setup_with_constructor(prj: &TestProject) -> String {
    prj.add_source(
        "TupleArrayConstructorContract",
        r#"
struct Point {
uint256 x;
uint256 y;
}

contract TupleArrayConstructorContract {
constructor(Point[] memory _points) {}
}
"#,
    )
    .unwrap();

    "src/TupleArrayConstructorContract.sol:TupleArrayConstructorContract".to_string()
}

/// configures the `TestProject` with the given closure and calls the `forge create` command
fn create_on_chain<F>(
    network_args: Option<Vec<String>>,
    constructor_args: Option<Vec<String>>,
    prj: TestProject,
    mut cmd: TestCommand,
    f: F,
    expected: impl IntoData,
) where
    F: FnOnce(&TestProject) -> String,
{
    if let Some(network_args) = network_args {
        let contract_path = f(&prj);

        cmd.arg("create")
            .arg("--resolc")
            .arg("--legacy")
            .arg("--broadcast")
            .args(network_args)
            .arg(contract_path);

        if let Some(constructor_args) = constructor_args {
            cmd.args(constructor_args);
        }
        cmd.assert_success().stdout_eq(expected);
    }
}

fn westend_assethub_args() -> Option<Vec<String>> {
    Some(
        [
            "--rpc-url".to_string(),
            network_rpc_key("westend_assethub")?,
            "--private-key".to_string(),
            network_private_key("westend_assethub")?,
        ]
        .to_vec(),
    )
}

fn localnode_args() -> Option<Vec<String>> {
    Some(
        [
            "--rpc-url".to_string(),
            PolkadotNode::http_endpoint().to_string(),
            "--private-key".to_string(),
            PolkadotNode::dev_accounts().next().unwrap().1.to_string(),
        ]
        .to_vec(),
    )
}

// These tests require setting the following environment variables:
// - `WESTEND_ASSETHUB_RPC_URL`: The RPC endpoint for the Westend Asset Hub, e.g.,
//  `https://westend-asset-hub-eth-rpc.polkadot.io`.
// - `WESTEND_ASSETHUB_PRIVATE_KEY`: The private key of the account that will be used to send
//   requests.
//
// Ensure these variables are set before running the tests to enable proper interaction with the
// Westend AssetHub.
// tests `forge` create on westend if correct env vars are set
forgetest_serial!(can_create_simple_on_westend_assethub, |prj, cmd| {
    create_on_chain(
        westend_assethub_args(),
        None,
        prj,
        cmd,
        setup_with_simple_remapping,
        CREATE_RESPONSE_PATTERN,
    );
});

// tests `forge` create on westend if correct env vars are set
forgetest_serial!(can_create_oracle_on_westend_assethub, |prj, cmd| {
    create_on_chain(westend_assethub_args(), None, prj, cmd, setup_oracle, CREATE_RESPONSE_PATTERN);
});

// tests that we can deploy with constructor args
forgetest_serial!(can_create_with_constructor_args_on_westend_assethub, |prj, cmd| {
    create_on_chain(
        westend_assethub_args(),
        Some(vec!["--constructor-args".to_string(), "[(1,2), (2,3), (3,4)]".to_string()]),
        prj,
        cmd,
        setup_with_constructor,
        CREATE_RESPONSE_PATTERN,
    );
});

// These tests require `substrate-node` and the Ethereum RPC proxy:
//
// ```bash
// git clone https://github.com/paritytech/polkadot-sdk
// cd polkadot-sdk
// cargo build --release --bin substrate-node
//
// cargo install pallet-revive-eth-rpc
// ```
//
// Ensure that both binaries are available in your system's PATH and are version-compatible.
forgetest_serial!(can_create_simple_on_polkadot_localnode, |prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        create_on_chain(
            localnode_args(),
            None,
            prj,
            cmd,
            setup_with_simple_remapping,
            CREATE_RESPONSE_PATTERN,
        );
    }
});

forgetest_serial!(can_create_oracle_on_polkadot_localnode, |prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        create_on_chain(localnode_args(), None, prj, cmd, setup_oracle, CREATE_RESPONSE_PATTERN);
    }
});

forgetest_serial!(can_create_with_constructor_args_on_polkadot_localnode, |prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        create_on_chain(
            localnode_args(),
            Some(vec!["--constructor-args".to_string(), "[(1,2), (2,3), (3,4)]".to_string()]),
            prj,
            cmd,
            setup_with_constructor,
            CREATE_RESPONSE_PATTERN,
        );
    }
});
