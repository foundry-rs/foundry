use alloy_consensus::{SignableTransaction, TxEip1559, transaction::SignerRecoverable};
use alloy_network::{ReceiptResponse, TxSignerSync, eip2718::Encodable2718};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, hex};
use alloy_provider::Provider;
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
    address constant BASE_FEE_VAULT = 0x4200000000000000000000000000000000000019;
    address constant L1_FEE_VAULT = 0x420000000000000000000000000000000000001A;
    address constant OPERATOR_FEE_VAULT = 0x420000000000000000000000000000000000001b;
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
        vm.store(L1_BLOCK, bytes32(uint256(8)), bytes32((uint256(1_000_000) << 64) | 7));
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
        uint256 baseFeeVaultBalance = BASE_FEE_VAULT.balance;
        uint256 l1FeeVaultBalance = L1_FEE_VAULT.balance;
        uint256 operatorFeeVaultBalance = OPERATOR_FEE_VAULT.balance;

        vm.prank(caller);
        (bool success,) = address(sink).call(hex"deadbeef");

        require(success, "call failed");
        require(caller.balance == 1 ether, "caller was charged");
        require(BASE_FEE_VAULT.balance == baseFeeVaultBalance, "base fee vault was credited");
        require(L1_FEE_VAULT.balance == l1FeeVaultBalance, "L1 fee vault was credited");
        require(
            OPERATOR_FEE_VAULT.balance == operatorFeeVaultBalance,
            "operator fee vault was credited"
        );
    }

    function test_create_does_not_credit_fee_vaults() public {
        uint256 baseFeeVaultBalance = BASE_FEE_VAULT.balance;
        uint256 l1FeeVaultBalance = L1_FEE_VAULT.balance;
        uint256 operatorFeeVaultBalance = OPERATOR_FEE_VAULT.balance;

        new Sink();

        require(BASE_FEE_VAULT.balance == baseFeeVaultBalance, "base fee vault was credited");
        require(L1_FEE_VAULT.balance == l1FeeVaultBalance, "L1 fee vault was credited");
        require(
            OPERATOR_FEE_VAULT.balance == operatorFeeVaultBalance,
            "operator fee vault was credited"
        );
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

forgetest!(base_isolated_snapshot_does_not_disable_broadcast_fees, |prj, cmd| {
    let signer = PrivateKeySigner::from_bytes(&B256::with_last_byte(1)).unwrap();
    let recipient = Address::with_last_byte(0x43);
    let mut transaction = TxEip1559 {
        chain_id: 8453,
        gas_limit: 21_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: TxKind::Call(recipient),
        value: U256::from(23),
        ..Default::default()
    };
    let signature = signer.sign_transaction_sync(&mut transaction).unwrap();
    let mut raw = Vec::new();
    transaction.into_signed(signature).eip2718_encode(&mut raw);

    prj.add_test(
        "BaseIsolatedSnapshotFees.t.sol",
        &format!(
            r#"
interface Vm {{
    function broadcastRawTransaction(bytes calldata data) external;
    function deal(address account, uint256 balance) external;
    function fee(uint256 newBasefee) external;
    function revertToState(uint256 snapshotId) external returns (bool);
    function revertToStateAndDelete(uint256 snapshotId) external returns (bool);
    function snapshotState() external returns (uint256);
    function store(address target, bytes32 slot, bytes32 value) external;
}}

contract BaseIsolatedSnapshotFeesTest {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant L1_BLOCK = 0x4200000000000000000000000000000000000015;
    address constant BASE_FEE_VAULT = 0x4200000000000000000000000000000000000019;
    address constant L1_FEE_VAULT = 0x420000000000000000000000000000000000001A;
    address constant OPERATOR_FEE_VAULT = 0x420000000000000000000000000000000000001b;

    function setUp() public {{
        vm.fee(1 gwei);
        vm.store(L1_BLOCK, bytes32(uint256(1)), bytes32(uint256(0x28f53a5b)));
        vm.store(
            L1_BLOCK,
            bytes32(uint256(3)),
            bytes32(uint256(0x08dd00101c120000000000000002))
        );
        vm.store(L1_BLOCK, bytes32(uint256(7)), bytes32(uint256(0x0240f4f5)));
        vm.store(L1_BLOCK, bytes32(uint256(8)), bytes32((uint256(1_000_000) << 64) | 7));
    }}

    function test_snapshot_from_isolated_helper_does_not_disable_broadcast_fees() public {{
        address sender = {sender};
        address recipient = {recipient};
        vm.deal(sender, 1 ether);

        uint256 snapshot = this.snapshotInHelper();
        require(vm.revertToState(snapshot), "snapshot revert failed");
        vm.broadcastRawTransaction(hex"{raw}");

        require(recipient.balance == 23, "transaction value not transferred");
        require(sender.balance < 1 ether - 23, "transaction fees not charged");
    }}

    function snapshotInHelper() external returns (uint256) {{
        return vm.snapshotState();
    }}

    function test_revert_inside_isolated_helper_does_not_credit_fee_vaults() public {{
        assertRevertDoesNotCreditFeeVaults(false);
    }}

    function test_revert_and_delete_inside_isolated_helper_does_not_credit_fee_vaults() public {{
        assertRevertDoesNotCreditFeeVaults(true);
    }}

    function assertRevertDoesNotCreditFeeVaults(bool deleteSnapshot) internal {{
        this.setFeeInHelper(2 gwei);
        require(block.basefee == 2 gwei, "fee override not set");
        uint256 baseFeeVaultBalance = BASE_FEE_VAULT.balance;
        uint256 l1FeeVaultBalance = L1_FEE_VAULT.balance;
        uint256 operatorFeeVaultBalance = OPERATOR_FEE_VAULT.balance;
        uint256 snapshot = vm.snapshotState();

        this.revertInHelper(snapshot, deleteSnapshot);

        require(block.basefee == 2 gwei, "fee override not preserved after helper");
        require(BASE_FEE_VAULT.balance == baseFeeVaultBalance, "base fee vault was credited");
        require(L1_FEE_VAULT.balance == l1FeeVaultBalance, "L1 fee vault was credited");
        require(
            OPERATOR_FEE_VAULT.balance == operatorFeeVaultBalance,
            "operator fee vault was credited"
        );
    }}

    function setFeeInHelper(uint256 basefee) external {{
        vm.fee(basefee);
    }}

    function revertInHelper(uint256 snapshot, bool deleteSnapshot) external {{
        bool success = deleteSnapshot
            ? vm.revertToStateAndDelete(snapshot)
            : vm.revertToState(snapshot);
        require(success, "snapshot revert failed");
        require(block.basefee == 2 gwei, "fee override not preserved inside helper");
    }}
}}
"#,
            sender = signer.address(),
            recipient = recipient,
            raw = hex::encode(raw),
        ),
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
        "BaseIsolatedSnapshotFeesTest",
    ])
    .assert_success();
});

forgetest_async!(base_fork_isolated_snapshot_fee_tracks_roll, |prj, cmd| {
    let (api, handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Azul.into()))).await;
    let provider = handle.http_provider();

    api.anvil_set_next_block_base_fee_per_gas(U256::from(1_000_000_000)).await.unwrap();
    api.mine_one().await.unwrap();
    let first_block = provider.get_block_number().await.unwrap();
    api.anvil_set_next_block_base_fee_per_gas(U256::from(2_000_000_000)).await.unwrap();
    api.mine_one().await.unwrap();
    let second_block = provider.get_block_number().await.unwrap();

    prj.add_test(
        "BaseForkIsolatedSnapshotFee.t.sol",
        &format!(
            r#"
interface Vm {{
    function createSelectFork(string calldata url, uint256 blockNumber) external returns (uint256);
    function fee(uint256 newBasefee) external;
    function revertToState(uint256 snapshotId) external returns (bool);
    function revertToStateAndDelete(uint256 snapshotId) external returns (bool);
    function rollFork(uint256 blockNumber) external;
    function snapshotState() external returns (uint256);
}}

contract BaseForkIsolatedSnapshotFeeTest {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    string constant RPC = "{rpc}";
    uint256 constant FIRST_BLOCK = {first_block};
    uint256 constant SECOND_BLOCK = {second_block};

    function test_implicit_fee_follows_fork_roll() public {{
        vm.createSelectFork(RPC, FIRST_BLOCK);
        for (uint256 i; i < 2; ++i) {{
            uint256 snapshot = vm.snapshotState();
            this.revertInHelper(snapshot, i == 1);
            require(block.basefee == 1 gwei, "snapshot base fee not restored");
            vm.rollFork(SECOND_BLOCK);
            require(block.basefee == 2 gwei, "old base fee pinned after fork roll");
            vm.rollFork(FIRST_BLOCK);
        }}
    }}

    function test_explicit_fee_survives_fork_roll() public {{
        vm.createSelectFork(RPC, FIRST_BLOCK);
        vm.fee(3 gwei);
        for (uint256 i; i < 2; ++i) {{
            uint256 snapshot = vm.snapshotState();
            this.revertInHelper(snapshot, i == 1);
            vm.rollFork(SECOND_BLOCK);
            require(block.basefee == 3 gwei, "explicit fee lost after fork roll");
            vm.rollFork(FIRST_BLOCK);
        }}
    }}

    function test_repeated_restore_keeps_implicit_fee_fork() public {{
        assertRepeatedRestore(false);
    }}

    function test_repeated_restore_and_delete_keeps_implicit_fee_fork() public {{
        assertRepeatedRestore(true);
    }}

    function assertRepeatedRestore(bool deleteSnapshot) internal {{
        vm.createSelectFork(RPC, FIRST_BLOCK);
        uint256 firstSnapshot = vm.snapshotState();
        vm.rollFork(SECOND_BLOCK);
        this.revertInHelper(firstSnapshot, deleteSnapshot);
        require(block.basefee == 1 gwei, "first snapshot base fee not restored");

        uint256 secondSnapshot = vm.snapshotState();
        this.revertInHelper(secondSnapshot, deleteSnapshot);
        require(block.basefee == 1 gwei, "implicit base fee overwritten on second revert");
    }}

    function revertInHelper(uint256 snapshot, bool deleteSnapshot) external {{
        bool success = deleteSnapshot
            ? vm.revertToStateAndDelete(snapshot)
            : vm.revertToState(snapshot);
        require(success, "snapshot revert failed");
    }}
}}
"#,
            rpc = handle.http_endpoint(),
        ),
    );

    cmd.args([
        "test",
        "--isolate",
        "--network",
        "base",
        "--hardfork",
        "base:Azul",
        "--chain-id",
        "8453",
        "--match-contract",
        "BaseForkIsolatedSnapshotFeeTest",
    ])
    .assert_success();
});

forgetest_async!(base_fork_isolated_inactive_hash_roll_charges_replayed_fees, |prj, cmd| {
    let (api, handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Azul.into()))).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let sender = wallets[0].address();
    let fresh_block = provider.get_block_number().await.unwrap();

    let mut marker = TxEip1559 {
        chain_id: 8453,
        gas_limit: 21_000,
        max_fee_per_gas: 3_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::with_last_byte(0x44)),
        value: U256::ONE,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut marker).unwrap();
    let mut marker_raw = Vec::new();
    marker.into_signed(signature).eip2718_encode(&mut marker_raw);

    let mut target = TxEip1559 {
        chain_id: 8453,
        gas_limit: 21_000,
        max_fee_per_gas: 3_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::with_last_byte(0x45)),
        value: U256::ONE,
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut target).unwrap();
    let mut target_raw = Vec::new();
    target.into_signed(signature).eip2718_encode(&mut target_raw);

    api.anvil_set_auto_mine(false).await.unwrap();
    let marker_pending = provider.send_raw_transaction(&marker_raw).await.unwrap();
    let target_pending = provider.send_raw_transaction(&target_raw).await.unwrap();
    let target_hash = *target_pending.tx_hash();
    api.mine_one().await.unwrap();
    let marker_receipt = marker_pending.get_receipt().await.unwrap();
    let target_receipt = target_pending.get_receipt().await.unwrap();
    api.anvil_set_auto_mine(true).await.unwrap();
    assert_eq!(marker_receipt.transaction_index(), Some(0));
    assert_eq!(target_receipt.transaction_index(), Some(1));
    let expected_sender_balance = provider.get_balance(sender).await.unwrap();

    prj.add_test(
        "BaseForkIsolatedReplayFees.t.sol",
        &format!(
            r#"
interface Vm {{
    function createFork(string calldata url, uint256 blockNumber) external returns (uint256);
    function createSelectFork(string calldata url, uint256 blockNumber)
        external
        returns (uint256);
    function rollFork(uint256 forkId, bytes32 transaction) external;
    function selectFork(uint256 forkId) external;
}}

contract BaseForkIsolatedReplayFeesTest {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    string constant RPC = "{rpc}";
    address constant SENDER = {sender};
    uint256 constant FRESH_BLOCK = {fresh_block};
    bytes32 constant TARGET_HASH = {target_hash};
    uint256 constant EXPECTED_SENDER_BALANCE = {expected_sender_balance};

    function test_fork_inactive_hash_roll_from_isolated_helper_charges_replay_fees() public {{
        vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 inactive = vm.createFork(RPC, FRESH_BLOCK);

        this.rollInactiveFork(inactive);
        vm.selectFork(inactive);

        require(SENDER.balance == EXPECTED_SENDER_BALANCE, "replay did not charge sender fees");
    }}

    function rollInactiveFork(uint256 forkId) external {{
        vm.rollFork(forkId, TARGET_HASH);
    }}
}}
"#,
            rpc = handle.http_endpoint(),
        ),
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
        "BaseForkIsolatedReplayFeesTest",
    ])
    .assert_success();
});
