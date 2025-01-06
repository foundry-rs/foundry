//! Contains various tests for checking the `forge create` subcommand

use crate::{
    constants::*,
    utils::{self, EnvExternalities},
};
use alloy_primitives::{hex, Address};
use anvil::{spawn, NodeConfig};
use foundry_compilers::artifacts::{remappings::Remapping, BytecodeHash};
use foundry_config::Config;
use foundry_test_utils::{
    forgetest, forgetest_async,
    snapbox::IntoData,
    str,
    util::{OutputExt, TestCommand, TestProject},
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
    let config = Config {
        remappings: vec![Remapping::from_str("remapping/=lib/remapping/").unwrap().into()],
        libraries: vec![format!("remapping/MyLib.sol:MyLib:{:?}", Address::random())],
        ..Default::default()
    };
    prj.write_config(config);

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
    let config = Config {
        libraries: vec![format!(
            "./src/libraries/ChainlinkTWAP.sol:ChainlinkTWAP:{:?}",
            Address::random()
        )],
        ..Default::default()
    };
    prj.write_config(config);

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
fn create_on_chain<F>(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand, f: F)
where
    F: FnOnce(&TestProject) -> String,
{
    if let Some(info) = info {
        let contract_path = f(&prj);

        let output = cmd
            .arg("create")
            .args(info.create_args())
            .arg(contract_path)
            .assert_success()
            .get_output()
            .stdout_lossy();
        let _address = utils::parse_deployed_address(output.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {output}"));
    }
}

// tests `forge` create on goerli if correct env vars are set
forgetest!(can_create_simple_on_goerli, |prj, cmd| {
    create_on_chain(EnvExternalities::goerli(), prj, cmd, setup_with_simple_remapping);
});

// tests `forge` create on goerli if correct env vars are set
forgetest!(can_create_oracle_on_goerli, |prj, cmd| {
    create_on_chain(EnvExternalities::goerli(), prj, cmd, setup_oracle);
});

// tests `forge` create on mumbai if correct env vars are set
forgetest!(can_create_oracle_on_mumbai, |prj, cmd| {
    create_on_chain(EnvExternalities::mumbai(), prj, cmd, setup_oracle);
});

// tests that we can deploy the template contract
forgetest_async!(can_create_template_contract, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // explicitly byte code hash for consistent checks
    let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
    prj.write_config(config);

    // Dry-run without the `--broadcast` flag
    cmd.forge_fuse().args([
        "create",
        format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
        "--rpc-url",
        rpc.as_str(),
        "--private-key",
        pk.as_str(),
    ]);

    // Dry-run
    cmd.assert().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Contract: Counter
Transaction: {
  "from": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
  "to": null,
  "maxFeePerGas": "0x77359401",
  "maxPriorityFeePerGas": "0x1",
  "gas": "0x17575",
  "input": "[..]",
  "nonce": "0x0",
  "chainId": "0x7a69"
}
ABI: [
  {
    "type": "function",
    "name": "increment",
    "inputs": [],
    "outputs": [],
    "stateMutability": "nonpayable"
  },
  {
    "type": "function",
    "name": "number",
    "inputs": [],
    "outputs": [
      {
        "name": "",
        "type": "uint256",
        "internalType": "uint256"
      }
    ],
    "stateMutability": "view"
  },
  {
    "type": "function",
    "name": "setNumber",
    "inputs": [
      {
        "name": "newNumber",
        "type": "uint256",
        "internalType": "uint256"
      }
    ],
    "outputs": [],
    "stateMutability": "nonpayable"
  }
]


"#]]);

    // Dry-run with `--json` flag
    cmd.arg("--json").assert().stdout_eq(
        str![[r#"
{
  "contract": "Counter",
  "transaction": {
    "from": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
    "to": null,
    "maxFeePerGas": "0x77359401",
    "maxPriorityFeePerGas": "0x1",
    "gas": "0x17575",
    "input": "[..]",
    "nonce": "0x0",
    "chainId": "0x7a69"
  },
  "abi": [
    {
      "type": "function",
      "name": "increment",
      "inputs": [],
      "outputs": [],
      "stateMutability": "nonpayable"
    },
    {
      "type": "function",
      "name": "number",
      "inputs": [],
      "outputs": [
        {
          "name": "",
          "type": "uint256",
          "internalType": "uint256"
        }
      ],
      "stateMutability": "view"
    },
    {
      "type": "function",
      "name": "setNumber",
      "inputs": [
        {
          "name": "newNumber",
          "type": "uint256",
          "internalType": "uint256"
        }
      ],
      "outputs": [],
      "stateMutability": "nonpayable"
    }
  ]
}

"#]]
        .is_json(),
    );

    cmd.forge_fuse().args([
        "create",
        format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
        "--rpc-url",
        rpc.as_str(),
        "--private-key",
        pk.as_str(),
        "--broadcast",
    ]);

    cmd.assert().stdout_eq(str![[r#"
No files changed, compilation skipped
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3
[TX_HASH]

"#]]);
});

// tests that we can deploy the template contract
forgetest_async!(can_create_using_unlocked, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let dev = handle.dev_accounts().next().unwrap();

    // explicitly byte code hash for consistent checks
    let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
    prj.write_config(config);

    cmd.forge_fuse().args([
        "create",
        format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
        "--rpc-url",
        rpc.as_str(),
        "--from",
        format!("{dev:?}").as_str(),
        "--unlocked",
        "--broadcast",
    ]);

    cmd.assert().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3
[TX_HASH]

"#]]);

    cmd.assert().stdout_eq(str![[r#"
No files changed, compilation skipped
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
[TX_HASH]

"#]]);
});

// tests that we can deploy with constructor args
forgetest_async!(can_create_with_constructor_args, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // explicitly byte code hash for consistent checks
    let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
    prj.write_config(config);

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
        .args([
            "create",
            "./src/ConstructorContract.sol:ConstructorContract",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--constructor-args",
            "My Constructor",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3
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
        .args([
            "create",
            "./src/TupleArrayConstructorContract.sol:TupleArrayConstructorContract",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--constructor-args",
            "[(1,2), (2,3), (3,4)]",
        ])
        .assert()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
[TX_HASH]

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/6332>
forgetest_async!(can_create_and_call, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // explicitly byte code hash for consistent checks
    let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
    prj.write_config(config);

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
        .args([
            "create",
            "./src/UniswapV2Swap.sol:UniswapV2Swap",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to pure
 [FILE]:6:5:
  |
6 |     function pairInfo() public view returns (uint reserveA, uint reserveB, uint totalSupply) {
  |     ^ (Relevant source part starts here and spans across multiple lines).

Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3
[TX_HASH]

"#]]);
});
