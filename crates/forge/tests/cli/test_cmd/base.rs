use alloy_consensus::transaction::SignerRecoverable;
use alloy_network::eip2718::Encodable2718;
use alloy_primitives::{B256, Bytes, hex};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use anvil::{NodeConfig, spawn};
use foundry_evm::{
    core::evm::{BaseEvmNetwork, TxEnvelopeFor},
    hardforks::BaseUpgrade,
};
use foundry_test_utils::util::OutputExt;
use serde_json::json;

forgetest!(base_azul_excludes_beryl_precompiles, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Azul",
        "--chain-id",
        "8453",
        "--match-test",
        "test_azul_excludes_beryl_precompiles",
    ])
    .assert_success();
});

forgetest!(base_defaults_to_azul, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--chain-id",
        "8453",
        "--match-test",
        "test_azul_excludes_beryl_precompiles",
    ])
    .assert_success();
});

forgetest!(base_beryl_precompiles_and_nested_evm, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    let stdout = cmd
        .args([
            "test",
            "--network",
            "base",
            "--hardfork",
            "base:Beryl",
            "--chain-id",
            "8453",
            "--match-test",
            "test_beryl",
            "-vvvv",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("ActivationRegistry"), "{stdout}");
    assert!(stdout.contains("B20Factory"), "{stdout}");
});

forgetest!(base_list_accepts_base_network, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--chain-id",
        "8453",
        "--list",
    ])
    .assert_success();
});

// Stateful Base precompile calls must work against a forked endpoint, not just locally: read-only
// ActivationRegistry/B20 calls already passed while `activate`/`createB20` reverted.
forgetest_async!(base_fork_allows_stateful_precompile_writes, |prj, cmd| {
    let (_api, handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()))).await;

    prj.add_test("BaseForkWrites.t.sol", include_str!("../../fixtures/BaseForkWrites.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--fork-url",
        &handle.http_endpoint(),
        "--match-contract",
        "BaseForkWritesTest",
        "-vvvv",
    ])
    .assert_success();
});

forgetest!(base_local_allows_stateful_precompile_writes, |prj, cmd| {
    prj.add_test("BaseForkWrites.t.sol", include_str!("../../fixtures/BaseForkWrites.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--chain-id",
        "8453",
        "--match-contract",
        "BaseForkWritesTest",
        "-vvvv",
    ])
    .assert_success();
});

forgetest!(base_script_uses_native_network, |prj, cmd| {
    let script = prj.add_script(
        "BaseScript.s.sol",
        r#"
interface IActivationRegistry {
    function admin() external view returns (address);
}

contract BaseScript {
    address constant ACTIVATION_REGISTRY = 0x8453000000000000000000000000000000000001;
    address constant MAINNET_BERYL_ADMIN = 0xcE3a3bEE7E72E2A24079f3c0Cb3b97740ED425A9;

    function run() external view {
        require(
            IActivationRegistry(ACTIVATION_REGISTRY).admin() == MAINNET_BERYL_ADMIN,
            "Base EVM not selected"
        );
    }
}
   "#,
    );

    cmd.arg("script")
        .arg(script)
        .args(["--network", "base", "--hardfork", "base:Beryl", "--chain-id", "8453"])
        .assert_success();
});

forgetest!(base_execute_transaction_rejects_eip8130, |prj, cmd| {
    let signer = PrivateKeySigner::from_bytes(&B256::with_last_byte(1)).unwrap();
    let mut envelope = json!({
        "type": "0x79",
        "tx": {
            "chainId": 8453, "sender": null, "payer": null,
            "nonceKey": "0x0", "nonceSequence": 0,
            "validAfter": 0, "validBefore": 0,
            "maxPriorityFeePerGas": "0x0", "maxFeePerGas": "0x3b9aca00",
            "gasLimit": 200000, "accountChanges": [], "calls": [], "metadata": "0x"
        },
        "senderAuth": "0x", "payerAuth": "0x"
    });
    let unsigned =
        serde_json::from_value::<TxEnvelopeFor<BaseEvmNetwork>>(envelope.clone()).unwrap();
    let signature = signer
        .sign_hash_sync(&unsigned.as_eip8130().unwrap().tx().sender_signature_hash())
        .unwrap();
    envelope["senderAuth"] = json!(Bytes::from(signature.as_bytes().to_vec()));
    let signed = serde_json::from_value::<TxEnvelopeFor<BaseEvmNetwork>>(envelope).unwrap();
    assert_eq!(signed.recover_signer().unwrap(), signer.address());
    let mut raw = Vec::new();
    signed.network_encode(&mut raw);
    let raw = hex::encode(raw);
    let sender = signer.address();
    prj.add_test(
        "BaseExecuteTransaction.t.sol",
        &format!(
            r#"
interface Vm {{
    function deal(address account, uint256 balance) external;
    function getNonce(address account) external view returns (uint64);
    function _expectCheatcodeRevert(bytes calldata reason) external;
    function executeTransaction(bytes calldata rawTx) external;
}}

contract BaseExecuteTransactionTest {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_rejects_eip8130_without_state_changes() public {{
        address sender = {sender};
        vm.deal(sender, 1 ether);
        uint64 nonce = vm.getNonce(sender);
        vm._expectCheatcodeRevert("EIP-8130 transactions are not supported by vm.executeTransaction");
        vm.executeTransaction(hex"{raw}");
        require(sender.balance == 1 ether, "sender balance changed");
        require(vm.getNonce(sender) == nonce, "sender nonce changed");
        require(block.chainid == 8453, "execution chain changed");
    }}
}}
"#
        ),
    );
    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Zenith",
        "--chain-id",
        "8453",
        "--match-contract",
        "BaseExecuteTransactionTest",
    ])
    .assert_success();
});

forgetest!(base_isolated_calls_do_not_charge_callers, |prj, cmd| {
    prj.add_test(
        "BaseIsolatedFees.t.sol",
        r#"
interface Vm {
    function deal(address account, uint256 balance) external;
    function prank(address sender) external;
    function store(address target, bytes32 slot, bytes32 value) external;
}

contract Sink {
    uint256 public hits;

    fallback() external payable {
        ++hits;
    }
}

contract BaseIsolatedFeesTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant L1_BLOCK = 0x4200000000000000000000000000000000000015;
    Sink sink;

    function setUp() public {
        sink = new Sink();

        vm.store(L1_BLOCK, bytes32(uint256(1)), bytes32(uint256(0x28f53a5b)));
        vm.store(
            L1_BLOCK,
            bytes32(uint256(3)),
            bytes32(uint256(0x08dd00101c120000000000000002))
        );
        vm.store(L1_BLOCK, bytes32(uint256(7)), bytes32(uint256(0x0240f4f5)));
    }

    function test_zero_balance_caller_succeeds() public {
        address caller = address(uint160(uint256(keccak256("caller"))));

        vm.prank(caller);
        (bool success,) = address(sink).call(hex"deadbeef");

        require(success, "call failed");
        require(sink.hits() == 1, "target was not called");
    }

    function test_funded_caller_balance_is_unchanged() public {
        address caller = address(uint160(uint256(keccak256("caller"))));
        vm.deal(caller, 1 ether);

        vm.prank(caller);
        (bool success,) = address(sink).call(hex"deadbeef");

        require(success, "call failed");
        require(caller.balance == 1 ether, "caller was charged");
    }
}
"#,
    );

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Azul",
        "--chain-id",
        "8453",
        "--match-contract",
        "BaseIsolatedFeesTest",
    ])
    .assert_success();
});
