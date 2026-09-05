//! CLI tests for send commands.

use super::*;

casttest!(send_rejects_invalid_eip1559_fees_before_access_list, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    let stderr = cmd
        .cast_fuse()
        .args([
            "send",
            "0x0000000000000000000000000000000000000001",
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
});

// ensure receipt or code is required
casttest!(send_requires_to, |_prj, cmd| {
    cmd.args([
        "send",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: Must specify a recipient address or contract code to deploy

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/9918>
casttest!(send_7702_conflicts_with_create, |_prj, cmd| {
    cmd.args([
        "send", "--private-key", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" ,"--auth", "0xf85c827a6994f39fd6e51aad88f6f4ce6ab8827279cfffb922668001a03e1a66234e71242afcc7bc46c8950c3b2997b102db257774865f1232d2e7bf48a045e252dad189b27b2306792047745eba86bff0dd18aca813dbf3fba8c4e94576", "--create",  "0x60806040523373ffffffffffffffffffffffffffffffffffffffff163273ffffffffffffffffffffffffffffffffffffffff1614610072576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610069906100e5565b60405180910390fd5b3373ffffffffffffffffffffffffffffffffffffffff16ff5b5f82825260208201905092915050565b7f74782e6f726967696e203d3d206d73672e73656e6465720000000000000000005f82015250565b5f6100cf60178361008b565b91506100da8261009b565b602082019050919050565b5f6020820190508181035f8301526100fc816100c3565b905091905056fe"
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: EIP-7702 transactions can't be CREATE transactions and require a destination address

"#]]);
});

casttest!(send_eip7702, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "send",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--gas-limit",
        "100000",
        "--rpc-url",
        &endpoint,
    ])
    .assert_success()
    .stderr_eq(str![""]);

    cmd.cast_fuse()
        .args(["code", "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
0xef010070997970c51812dc3a010c7d01b50e0d17dc79c8

"#]]);
});

casttest!(send_eip7702_auth_disclosure_declined, |_prj, cmd| {
    cmd.args([
        "send",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .stdin("n\n")
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);
});

casttest!(send_eip7702_auth_disclosure_forced, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;

    cmd.args([
        "send",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--force",
        "--async",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]

"#]])
    .stderr_eq(str![""]);
});

casttest!(send_sponsor_hash_supports_address_auth, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test_tempo()).await;

    cmd.args([
        "send",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--force",
        "--tempo.print-sponsor-hash",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]

"#]])
    .stderr_eq(str![""]);
});

casttest!(batch_send_eip7702_auth_disclosure, async |_prj, cmd| {
    let args = [
        "batch-send",
        "--call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    ];

    cmd.args(args)
        .args(["--chain", "31337", "--rpc-url", "http://127.0.0.1:1"])
        .stdin("n\n")
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Building batch transaction with 1 call(s)...
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);

    let (_api, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    cmd.cast_fuse()
        .args(args)
        .args(["--force", "--async", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]

"#]])
        .stderr_eq(str![[r#"
Building batch transaction with 1 call(s)...

"#]]);
});

casttest!(send_eip7702_multiple_auth, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();

    // Create a pre-signed authorization using a different signer (account index 1)
    let signer: PrivateKeySigner =
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".parse().unwrap();
    // Anvil default chain_id is 31337
    let auth = Authorization {
        chain_id: U256::from(31337),
        // Delegate to account index 2
        address: address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        nonce: 0,
    };
    let signature = signer.sign_hash(&auth.signature_hash()).await.unwrap();
    let signed_auth = auth.into_signed(signature);
    let encoded_auth = hex::encode_prefixed(alloy_rlp::encode(&signed_auth));

    // Send transaction with multiple --auth flags: one address and one pre-signed authorization
    let output = cmd
        .args([
            "send",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "--auth",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            "--auth",
            &encoded_auth,
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--gas-limit",
            "100000",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Extract transaction hash from JSON output
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let tx_hash = json["transactionHash"].as_str().unwrap();

    // Use cast tx to verify multiple authorizations were included
    let tx_output = cmd
        .cast_fuse()
        .args(["tx", tx_hash, "--rpc-url", &endpoint, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let tx_envelope: serde_json::Value = serde_json::from_str(&tx_output).unwrap();
    let auth_list = tx_envelope["data"]["authorizationList"].as_array().unwrap();

    // Verify we have 2 authorizations
    assert_eq!(auth_list.len(), 2, "Expected 2 authorizations in the transaction");

    let field_output = cmd
        .cast_fuse()
        .args(["tx", tx_hash, "authorizationList", "--rpc-url", &endpoint, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let field_envelope: serde_json::Value = serde_json::from_str(field_output.trim()).unwrap();
    let field_auth_list = field_envelope["data"].as_array().unwrap();
    assert_eq!(field_auth_list.len(), 2, "Expected authorizationList field data to be an array");
});

// Test that multiple address-based authorizations are rejected
casttest!(send_eip7702_multiple_address_auth_rejected, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "send",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--auth",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: Multiple address-based authorizations provided. Only one address can be specified; use pre-signed authorizations (hex-encoded) for multiple authorizations.

"#]]);
});

casttest!(send_sync, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    let output = cmd
        .args([
            "send",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            "--value",
            "1",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--sync",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("transactionHash"));
    assert!(output.contains("blockNumber"));
    assert!(output.contains("gasUsed"));
});

// tests cast send gas estimate execution failure message contains decoded custom error
// <https://github.com/foundry-rs/foundry/issues/9789>
forgetest_async!(cast_send_estimate_gas_error, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "SimpleStorage",
        r#"
contract SimpleStorage {
    uint256 private storedValue;
    error AddressInsufficientBalance(address account, uint256 newValue);
    function setValue(uint256 _newValue) public {
        if (_newValue > 100) {
            revert AddressInsufficientBalance(msg.sender, _newValue);
        }
        storedValue = _newValue;
    }
}
   "#,
    );
    prj.add_script(
        "SimpleStorageScript",
        r#"
import "forge-std/Script.sol";
import {SimpleStorage} from "../src/SimpleStorage.sol";
contract SimpleStorageScript is Script {
    function run() public {
        vm.startBroadcast();
        new SimpleStorage();
        vm.stopBroadcast();
    }
}
   "#,
    );

    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "SimpleStorageScript",
    ])
    .assert_success();

    // Cache project selectors.
    cmd.forge_fuse().set_current_dir(prj.root());
    cmd.forge_fuse().args(["selectors", "cache"]).assert_success();

    // Assert cast send can decode custom error on estimate gas execution failure.
    cmd.cast_fuse()
        .args([
            "send",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "setValue(uint256)",
            "1000",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_failure().stderr_eq(str![[r#"
Error: Failed to estimate gas: server returned an error response: error code 3: execution reverted: custom error 0x6786ad34: 000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb9226600000000000000000000000000000000000000000000000000000000000003e8, data: "0x6786ad34000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb9226600000000000000000000000000000000000000000000000000000000000003e8"[..]

"#]]);
});

// Test that cast send --create works correctly with constructor arguments
// <https://github.com/foundry-rs/foundry/issues/10947>
forgetest_async!(cast_send_create_with_constructor_args, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Deploy a simple contract with constructor arguments
    // Contract source that takes constructor args
    prj.add_source(
        "ConstructorContract",
        r#"
contract ConstructorContract {
    uint256 public value;
    string public name;

    constructor(uint256 _value, string memory _name) {
        value = _value;
        name = _name;
    }

    function getValue() public view returns (uint256) {
        return value;
    }
}
"#,
    );

    // Compile to get bytecode
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get the compiled bytecode
    let bytecode_path = prj.root().join("out/ConstructorContract.sol/ConstructorContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    // Use cast send --create with constructor arguments
    let output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
            "constructor(uint256,string)",
            "42",
            "TestContract",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Extract the deployed contract address from output
    let lines: Vec<&str> = output.lines().collect();
    let mut address = None;
    for line in lines {
        if line.contains("contractAddress") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            address = Some(parts[1]);
            break;
        }
    }
    let address = address.expect("Contract address not found in output");

    // Verify the contract was deployed correctly by calling getValue()
    let value_output = cmd
        .cast_fuse()
        .args(["call", address, "getValue()", "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // The value should be 42 (0x2a in hex)
    assert!(
        value_output.contains("0x000000000000000000000000000000000000000000000000000000000000002a")
    );

    cmd.cast_fuse()
        .args(["--json", "call", address, "getValue()(uint256)", "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
[
  "42"
]

"#]]);
});

// Test edge case: empty constructor arguments
// <https://github.com/foundry-rs/foundry/issues/10947>
forgetest_async!(cast_send_create_empty_constructor, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Simple contract with no constructor arguments
    prj.add_source(
        "SimpleContract",
        r#"
contract SimpleContract {
    uint256 public constant VALUE = 42;
}
"#,
    );

    // Compile
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get bytecode
    let bytecode_path = prj.root().join("out/SimpleContract.sol/SimpleContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    // Deploy with empty constructor
    let output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
            "constructor()",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify deployment succeeded
    assert!(output.contains("contractAddress"));
});

// Test complex constructor arguments (multiple types)
// <https://github.com/foundry-rs/foundry/issues/10947>
forgetest_async!(cast_send_create_complex_constructor, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Contract with complex constructor
    prj.add_source(
        "ComplexContract",
        r#"
contract ComplexContract {
    address public owner;
    uint256[] public values;
    bool public active;

    constructor(address _owner, uint256[] memory _values, bool _active) {
        owner = _owner;
        values = _values;
        active = _active;
    }

    function getValuesLength() public view returns (uint256) {
        return values.length;
    }
}
"#,
    );

    // Compile
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get bytecode
    let bytecode_path = prj.root().join("out/ComplexContract.sol/ComplexContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    // Deploy with complex arguments
    let output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
            "constructor(address,uint256[],bool)",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "[1,2,3,4,5]",
            "true",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Extract deployed address
    let lines: Vec<&str> = output.lines().collect();
    let mut address = None;
    for line in lines {
        if line.contains("contractAddress") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                address = Some(parts[1]);
                break;
            }
        }
    }
    let address = address.expect("Contract address not found in output");

    // Verify the array length was set correctly
    let length_output = cmd
        .cast_fuse()
        .args(["call", address, "getValuesLength()", "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Should return 5 (0x5 in hex)
    assert!(
        length_output
            .contains("0x0000000000000000000000000000000000000000000000000000000000000005")
    );
});

// Test cast send with raw --data flag using encoded calldata
forgetest_async!(cast_send_with_data, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();

    // Deploy counter contract
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    // setNumber(111) encoded: selector 0x3fb5c1cb + uint256(111)
    let calldata = "0x3fb5c1cb000000000000000000000000000000000000000000000000000000000000006f";

    // Send tx using --data instead of sig+args
    cmd.cast_fuse()
        .args([
            "send",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--data",
            calldata,
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success();

    // Verify via trace that setNumber(111) was called
    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    cmd.cast_fuse()
        .args([
            "run",
            format!("{tx_hash}").as_str(),
            "-vvvvv",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::setNumber(111)
    ├─  storage changes:
    │   @ 0: 0 → 111
    └─ ← [Stop]


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);
});

casttest!(publish_raw_transaction, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let signer = handle.dev_wallets().next().unwrap();
    let raw = cmd
        .args([
            "mktx",
            "0x0000000000000000000000000000000000000001",
            "--private-key",
            &hex::encode(signer.to_bytes()),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();
    let hash = keccak256(hex::decode(&raw).unwrap());
    cmd.cast_fuse()
        .args(["publish", &raw, "--async", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(format!("{hash}\n"));
});
