//! Contains various tests for checking the `forge create` subcommand

use crate::utils::{self, network_private_key, network_rpc_key};
use alloy_primitives::Address;
use foundry_compilers::artifacts::{remappings::Remapping, BytecodeHash};
use foundry_test_utils::{
    forgetest, forgetest_async,
    revive::PolkadotHubNode,
    str,
    util::{OutputExt, TestProject},
    TestCommand,
};
use std::str::FromStr;

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
    function foo() public returns (uint256) {
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
    function foobar(uint256 a) public view returns (uint256) {
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
    function getPrice() public view returns (int latest) {
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
   function getLatestPrice(address base) public view returns (int256) {
        return 0;
   }
}
",
    )
    .unwrap();

    "src/Contract.sol:Contract".to_string()
}

/// configures the `TestProject` with the given closure and calls the `forge create` command
fn create_on_chain<F>(
    network_args: Option<Vec<String>>,
    prj: TestProject,
    mut cmd: TestCommand,
    f: F,
) where
    F: FnOnce(&TestProject) -> String,
{
    if let Some(network_args) = network_args {
        let contract_path = f(&prj);

        let output = cmd
            .arg("create")
            .arg("--revive")
            .arg("--legacy")
            .arg("--broadcast")
            .args(network_args)
            .arg(contract_path)
            .assert_success()
            .get_output()
            .stdout_lossy();
        let _address = utils::parse_deployed_address(output.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {output}"));
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
            PolkadotHubNode::http_endpoint().to_string(),
            "--private-key".to_string(),
            PolkadotHubNode::dev_accounts().next().unwrap().1.to_string(),
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
forgetest!(can_create_simple_on_westend_assethub, |prj, cmd| {
    create_on_chain(westend_assethub_args(), prj, cmd, setup_with_simple_remapping);
});

// tests `forge` create on westend if correct env vars are set
forgetest!(can_create_oracle_on_westend_assethub, |prj, cmd| {
    create_on_chain(westend_assethub_args(), prj, cmd, setup_oracle);
});

// tests that we can deploy with constructor args
forgetest_async!(can_create_with_constructor_args_on_westend_assethub, |prj, cmd| {
    if let Some(network_args) = westend_assethub_args() {
        foundry_test_utils::util::initialize(prj.root());

        // explicitly byte code hash for consistent checks
        prj.update_config(|c| c.bytecode_hash = BytecodeHash::None);

        prj.add_source(
            "ConstructorContract",
            r#"
contract ConstructorContract {
    string public name;

    constructor(string memory _name) {
        name = _name;
    }
}
"#,
        )
        .unwrap();

        cmd.forge_fuse()
            .arg("create")
            .arg("--revive")
            .arg("--legacy")
            .arg("--broadcast")
            .arg("./src/ConstructorContract.sol:ConstructorContract")
            .args(&network_args)
            .args(["--constructor-args", "My Constructor"])
            .assert_success()
            .stdout_eq(str![[r#"
[COMPILING_FILES] with [REVIVE_VERSION]
[REVIVE_VERSION] [ELAPSED]
Compiler run successful!
Deployer: [..]
Deployed to: [..]
[TX_HASH]

"#]]);

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

        cmd.forge_fuse()
            .arg("create")
            .arg("--revive")
            .arg("--legacy")
            .arg("--broadcast")
            .arg("./src/TupleArrayConstructorContract.sol:TupleArrayConstructorContract")
            .args(network_args)
            .args(["--constructor-args", "[(1,2), (2,3), (3,4)]"])
            .assert()
            .stdout_eq(str![[r#"
[COMPILING_FILES] with [REVIVE_VERSION]
[REVIVE_VERSION] [ELAPSED]
Compiler run successful!
Deployer: [..]
Deployed to: [..]
[TX_HASH]

"#]]);
    }
});

// <https://github.com/foundry-rs/foundry/issues/6332>
forgetest_async!(can_create_and_call_on_westend_assethub, |prj, cmd| {
    if let Some(network_args) = westend_assethub_args() {
        foundry_test_utils::util::initialize(prj.root());

        // explicitly byte code hash for consistent checks
        prj.update_config(|c| c.bytecode_hash = BytecodeHash::None);

        prj.add_source(
            "UniswapV2Swap",
            r#"
contract UniswapV2Swap {

    function pairInfo() public view returns (uint reserveA, uint reserveB, uint totalSupply) {
       (reserveA, reserveB, totalSupply) = (0,0,0);
    }

}
"#,
        )
        .unwrap();
        cmd.forge_fuse()
            .arg("create")
            .arg("--revive")
            .arg("--legacy")
            .arg("--broadcast")
            .arg("./src/UniswapV2Swap.sol:UniswapV2Swap")
            .args(network_args)
            .assert_success()
            .stdout_eq(str![[r#"
[COMPILING_FILES] with [REVIVE_VERSION]
[REVIVE_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to pure
 [FILE]:6:5:
  |
6 |     function pairInfo() public view returns (uint reserveA, uint reserveB, uint totalSupply) {
  |     ^ (Relevant source part starts here and spans across multiple lines).

Deployer: [..]
Deployed to: [..]
[TX_HASH]

"#]]);
    }
});

forgetest_async!(can_create_simple_on_localnode, |prj, cmd| {
    if let Ok(_node) = PolkadotHubNode::start().await {
        create_on_chain(localnode_args(), prj, cmd, setup_with_simple_remapping);
    }
});
