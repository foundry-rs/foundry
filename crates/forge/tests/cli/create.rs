//! Contains various tests for checking the `forge create` subcommand

use crate::constants::*;
use alloy_primitives::hex;
use anvil::{NodeConfig, spawn};
use foundry_compilers::artifacts::BytecodeHash;
use foundry_test_utils::{forgetest, snapbox::IntoData, str, util::OutputExt};
use std::{fs, time::Duration};

#[forgetest]
fn create_rejects_unsupported_remote_sponsor(cmd: _) {
    cmd.args([
        "create",
        "src/Counter.sol:Counter",
        "--sponsor-url",
        "https://sponsor.tempo.xyz/tp_test",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: --sponsor-url is not supported by forge create; use --tempo.sponsor with --tempo.sponsor-signer or --tempo.sponsor-sig

"#]]);
}

// tests that we can deploy the template contract
#[forgetest_init]
async fn can_create_template_contract(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // explicitly byte code hash for consistent checks
    prj.update_config(|c| c.bytecode_hash = BytecodeHash::None);

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
  "gas": "0x241e7",
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
    "gas": "0x241e7",
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
}

// The deployment is only mined on the next interval tick.
#[forgetest_init]
async fn can_create_with_interval_mining(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) =
        spawn(NodeConfig::test().with_blocktime(Some(Duration::from_secs(1)))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    cmd.forge_fuse()
        .args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
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
Compiler run successful!
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3
[TX_HASH]

"#]]);
}

#[forgetest_init]
async fn create_rejects_from_signer_mismatch(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let mut wallets = handle.dev_wallets();
    let from = wallets.next().unwrap();
    let from_pk = hex::encode(from.credential().to_bytes());
    let signer = wallets.next().unwrap();
    let signer_pk = hex::encode(signer.credential().to_bytes());
    let from = from.address().to_string();
    let contract = format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}");
    let args = ["create", contract.as_str(), "--rpc-url", rpc.as_str(), "--broadcast"];

    // A signer that does not match `--from` is rejected before anything is sent.
    cmd.forge_fuse()
        .args(args)
        .args(["--from", &from, "--private-key", &signer_pk])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: the sender specified via `--from`/`ETH_FROM` (0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266) does not match the signer address (0x70997970C51812dc3A010C7d01b50e0d17dc79C8)

"#]]);
    assert!(api.transaction_count(signer.address(), None).await.unwrap().is_zero());

    // A signer that matches `--from` deploys as usual.
    cmd.forge_fuse()
        .args(args)
        .args(["--from", &from, "--private-key", &from_pk])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0x5FbDB2315678afecb367f032d93F642f64180aa3
[TX_HASH]

"#]]);

    // Unlocked deployments are sent from `--from` and ignore the resolved signer.
    cmd.forge_fuse()
        .args(args)
        .args(["--unlocked", "--from", &from, "--private-key", &signer_pk])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped
Deployer: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Deployed to: 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
[TX_HASH]

"#]]);
    assert!(api.transaction_count(signer.address(), None).await.unwrap().is_zero());
}

#[forgetest_init]
async fn create_rejects_invalid_eip1559_fees_before_access_list(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    let stderr = cmd
        .forge_fuse()
        .args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--access-list",
            "--gas-price",
            "1",
            "--priority-gas-price",
            "2",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr.contains("Error: max priority fee per gas (2) cannot exceed max fee per gas (1)"),
        "{stderr}"
    );
}

#[forgetest_init]
async fn create_resolves_tempo_expires_before_broadcast(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // explicitly byte code hash for consistent checks
    prj.update_config(|c| c.bytecode_hash = BytecodeHash::None);

    let assert = cmd
        .forge_fuse()
        .args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--tempo.expires",
            "30",
        ])
        .assert_success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("Transaction expires at unix timestamp "),
        "expected create to print resolved tempo expiry, got:\n{stderr}",
    );
    assert!(stdout.contains("Deployed to:"), "{stdout}");
}

#[forgetest_init]
async fn create_broadcasts_with_local_tempo_sponsor(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let wallets = handle.dev_wallets().take(2).collect::<Vec<_>>();
    let sender_key = hex::encode(wallets[0].credential().to_bytes());
    let sponsor_key =
        format!("private-key://{}", hex::encode_prefixed(wallets[1].credential().to_bytes()));
    let sponsor = format!("{:?}", wallets[1].address());

    prj.update_config(|config| config.bytecode_hash = BytecodeHash::None);

    let assert = cmd
        .forge_fuse()
        .args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
            "--rpc-url",
            &rpc,
            "--private-key",
            &sender_key,
            "--broadcast",
            "--tempo.fee-token",
            "PathUSD",
            "--tempo.sponsor",
            &sponsor,
            "--tempo.sponsor-signer",
            &sponsor_key,
        ])
        .assert_success();
    let output = assert.get_output();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Deployed to:"), "{stdout}");
    assert!(stderr.to_ascii_lowercase().contains(&format!("tempo sponsor: {sponsor}")), "{stderr}");
}

#[forgetest_init]
async fn create_rejects_tempo_access_key_before_broadcast(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    prj.update_config(|config| config.bytecode_hash = BytecodeHash::None);
    let stderr = cmd
        .forge_fuse()
        .args([
            "create",
            format!("./src/{TEMPLATE_CONTRACT}.sol:{TEMPLATE_CONTRACT}").as_str(),
            "--rpc-url",
            &rpc,
            "--tempo.access-key",
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
            "--tempo.root-account",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "--broadcast",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("Tempo access-key transactions cannot use CREATE"), "{stderr}");
}

// tests that we can deploy the template contract
#[forgetest_init]
async fn can_create_using_unlocked(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let dev = handle.dev_accounts().next().unwrap();

    // explicitly byte code hash for consistent checks
    prj.update_config(|c| c.bytecode_hash = BytecodeHash::None);

    // A matching Tempo Accounts entry must not change an ordinary Ethereum deployment into a
    // Tempo transaction.
    let tempo_home = tempfile::tempdir().unwrap();
    let wallet_dir = tempo_home.path().join("wallet");
    fs::create_dir_all(&wallet_dir).unwrap();
    let store = serde_json::json!({
        "tempo-cli.store": {
            "state": {
                "activeAccount": 0,
                "chainId": 31337,
                "accounts": [{"address": format!("{dev:?}")}],
                "accessKeys": [{
                    "access": format!("{dev:?}"),
                    "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
                    "chainId": 31337,
                    "keyType": "secp256k1",
                    "privateKey": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
                }],
            },
        },
    });
    fs::write(wallet_dir.join("store.json"), serde_json::to_vec(&store).unwrap()).unwrap();

    cmd.forge_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.args([
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
}

// tests that we can deploy with constructor args
#[forgetest_init]
async fn can_create_with_constructor_args(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

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
    );

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
    );

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
}

// <https://github.com/foundry-rs/foundry/issues/6332>
#[forgetest_init]
async fn can_create_and_call(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

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
    );

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
}

// <https://github.com/foundry-rs/foundry/issues/10156>
#[forgetest]
async fn should_err_if_no_bytecode(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();

    prj.add_source(
        "AbstractCounter.sol",
        r#"
abstract contract AbstractCounter {
    uint256 public number;

    function setNumberV1(uint256 newNumber) public {
        number = newNumber;
    }

    function incrementV1() public {
        number++;
    }
}
    "#,
    );

    cmd.args([
        "create",
        "./src/AbstractCounter.sol:AbstractCounter",
        "--rpc-url",
        rpc.as_str(),
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--broadcast",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: no bytecode found in bin object for AbstractCounter

"#]]);
}

// Tests that `forge create` fails when the deployment transaction reverts
// <https://github.com/foundry-rs/foundry/issues/13954>
#[forgetest]
async fn flaky_should_fail_on_reverted_deployment(prj: _, cmd: _) {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    prj.add_source(
        "RevertingContract.sol",
        r#"
contract RevertingContract {
    constructor() {
        revert("deployment failed");
    }
}
    "#,
    );

    // Use --gas-limit to bypass eth_estimateGas, which would reject the tx early.
    // This simulates chains that mine reverted txs (e.g. when gas is manually specified).
    cmd.args([
        "create",
        "./src/RevertingContract.sol:RevertingContract",
        "--rpc-url",
        rpc.as_str(),
        "--private-key",
        pk.as_str(),
        "--broadcast",
        "--gas-limit",
        "1000000",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: deployment transaction failed (receipt status 0): [..]

"#]]);
}
