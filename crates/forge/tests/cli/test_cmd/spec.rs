#[cfg(feature = "monad")]
use alloy_consensus::{SignableTransaction, TxEip1559};
#[cfg(feature = "monad")]
use alloy_network::{Ethereum, Network, ReceiptResponse, TransactionBuilder, TxSignerSync};
#[cfg(feature = "monad")]
use alloy_primitives::{Address, B256, TxKind, U256, address, hex, keccak256};
#[cfg(feature = "monad")]
use alloy_provider::Provider;
#[cfg(feature = "monad")]
use anvil::{NodeConfig, spawn};
#[cfg(feature = "monad")]
use axum::{Json, Router, routing::post};
use foundry_compilers::artifacts::EvmVersion;
#[cfg(feature = "monad")]
use foundry_evm::hardforks::MonadHardfork;
use foundry_evm::hardforks::{FoundryHardfork, TempoHardfork};
use foundry_test_utils::{rpc, util::OTHER_SOLC_VERSION};
#[cfg(feature = "monad")]
use serde_json::{Value, json};

#[cfg(feature = "monad")]
fn canonicalize_monad_reward_transaction(transaction: &mut Value, target_hash: &str) {
    let Some(transaction) = transaction.as_object_mut() else { return };
    if !transaction
        .get("hash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(target_hash))
    {
        return;
    }

    transaction.insert("gas".to_string(), json!("0x0"));
    transaction.insert("gasPrice".to_string(), json!("0x0"));
    transaction.insert("r".to_string(), json!("0x0"));
    transaction.insert("s".to_string(), json!("0x0"));
    transaction.insert("type".to_string(), json!("0x0"));
    transaction.insert("v".to_string(), json!("0x0"));
    transaction.remove("accessList");
    transaction.remove("maxFeePerGas");
    transaction.remove("maxPriorityFeePerGas");
    transaction.remove("yParity");
}

#[cfg(feature = "monad")]
fn canonicalize_monad_reward_receipt(receipt: &mut Value, target_hash: &str) {
    let Some(receipt) = receipt.as_object_mut() else { return };
    if !receipt
        .get("transactionHash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(target_hash))
    {
        return;
    }

    receipt.insert("cumulativeGasUsed".to_string(), json!("0x0"));
    receipt.insert("effectiveGasPrice".to_string(), json!("0x0"));
    receipt.insert("gasUsed".to_string(), json!("0x0"));
    receipt.insert("type".to_string(), json!("0x0"));
}

#[cfg(feature = "monad")]
async fn spawn_canonical_monad_reward_rpc(endpoint: String, target_hash: B256) -> String {
    let target_hash = target_hash.to_string();
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let endpoint = endpoint.clone();
            let target_hash = target_hash.clone();
            async move {
                let method = request["method"].as_str().unwrap();
                let mut response = reqwest::Client::new()
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();

                match method {
                    "eth_getTransactionByHash"
                    | "eth_getTransactionByBlockHashAndIndex"
                    | "eth_getTransactionByBlockNumberAndIndex" => {
                        canonicalize_monad_reward_transaction(
                            &mut response["result"],
                            &target_hash,
                        );
                    }
                    "eth_getBlockByHash" | "eth_getBlockByNumber" => {
                        if let Some(transactions) =
                            response["result"]["transactions"].as_array_mut()
                        {
                            for transaction in transactions {
                                canonicalize_monad_reward_transaction(transaction, &target_hash);
                            }
                        }
                    }
                    "eth_getTransactionReceipt" => {
                        canonicalize_monad_reward_receipt(&mut response["result"], &target_hash);
                    }
                    "eth_getBlockReceipts" => {
                        if let Some(receipts) = response["result"].as_array_mut() {
                            for receipt in receipts {
                                canonicalize_monad_reward_receipt(receipt, &target_hash);
                            }
                        }
                    }
                    _ => {}
                }

                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

#[cfg(feature = "monad")]
async fn rpc_request(endpoint: &str, method: &str, params: Value) -> Value {
    reqwest::Client::new()
        .post(endpoint)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[cfg(feature = "monad")]
fn monad_staking_validator_id_key(address: Address) -> U256 {
    let mut key = [0u8; 32];
    key[0] = 0x06;
    key[1..21].copy_from_slice(address.as_slice());
    U256::from_be_bytes(key)
}

#[cfg(feature = "monad")]
fn monad_staking_validator_key(namespace: u8, validator_id: u64, offset: u8) -> U256 {
    let mut key = [0u8; 32];
    key[0] = namespace;
    key[1..9].copy_from_slice(&validator_id.to_be_bytes());
    U256::from_be_bytes(key) + U256::from(offset)
}

#[cfg(feature = "monad")]
fn left_aligned_u64(value: u64) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&value.to_be_bytes());
    U256::from_be_bytes(bytes)
}

#[cfg(feature = "monad")]
fn address_and_flags(address: Address, flags: u64) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..20].copy_from_slice(address.as_slice());
    bytes[20..28].copy_from_slice(&flags.to_be_bytes());
    U256::from_be_bytes(bytes)
}

#[cfg(feature = "monad")]
fn storage_value(value: U256) -> B256 {
    B256::from(value.to_be_bytes::<32>())
}

// Test evm version switch during tests / scripts.
// <https://github.com/foundry-rs/foundry/issues/9840>
// <https://github.com/foundry-rs/foundry/issues/6228>
forgetest_init!(test_set_evm_version, |prj, cmd| {
    let endpoint = rpc::next_http_archive_rpc_url();
    prj.add_test(
        "TestEvmVersion.t.sol",
        &r#"
import {Test} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
}

interface ICreate2Deployer {
    function computeAddress(bytes32 salt, bytes32 codeHash) external view returns (address);
}

contract TestEvmVersion is Test {
    function test_evm_version() public {
        EvmVm evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));
        vm.createSelectFork("<rpc>");

        evm.setEvmVersion("istanbul");
        evm.getEvmVersion();

        // revert with NotActivated for istanbul
        vm.expectRevert();
        compute();

        evm.setEvmVersion("shanghai");
        evm.getEvmVersion();
        compute();

        // switch to Paris, expect revert with NotActivated
        evm.setEvmVersion("paris");
        vm.expectRevert();
        compute();
    }

    function compute() internal view {
        ICreate2Deployer(0x35Da41c476fA5c6De066f20556069096A1F39364).computeAddress(bytes32(0), bytes32(0));
    }
}
   "#.replace("<rpc>", &endpoint),
    );

    cmd.args(["test", "--mc", "TestEvmVersion", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/TestEvmVersion.t.sol:TestEvmVersion
[PASS] test_evm_version() ([GAS])
Traces:
  [..] TestEvmVersion::test_evm_version()
    ├─ [0] VM::createSelectFork("<rpc url>")
    │   └─ ← [Return] 0
    ├─ [0] VM::setEvmVersion("istanbul")
    │   └─ ← [Return]
    ├─ [0] VM::getEvmVersion() [staticcall]
    │   └─ ← [Return] "istanbul"
    ├─ [0] VM::expectRevert(custom error 0xf4844814)
    │   └─ ← [Return]
    ├─ [..] 0x35Da41c476fA5c6De066f20556069096A1F39364::computeAddress(0x0000000000000000000000000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   └─ ← [NotActivated] EvmError: NotActivated
    ├─ [0] VM::setEvmVersion("shanghai")
    │   └─ ← [Return]
    ├─ [0] VM::getEvmVersion() [staticcall]
    │   └─ ← [Return] "shanghai"
    ├─ [..] 0x35Da41c476fA5c6De066f20556069096A1F39364::computeAddress(0x0000000000000000000000000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   └─ ← [Return] 0x0f40d7B7669e3a6683EaB25358318fd42a9F2342
    ├─ [0] VM::setEvmVersion("paris")
    │   └─ ← [Return]
    ├─ [0] VM::expectRevert(custom error 0xf4844814)
    │   └─ ← [Return]
    ├─ [..] 0x35Da41c476fA5c6De066f20556069096A1F39364::computeAddress(0x0000000000000000000000000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000000) [staticcall]
    │   └─ ← [NotActivated] EvmError: NotActivated
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test evm version set in `setUp` is accounted in test.
    prj.add_test(
        "TestSetupEvmVersion.t.sol",
        &r#"
import {Test} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
}

interface ICreate2Deployer {
    function computeAddress(bytes32 salt, bytes32 codeHash) external view returns (address);
}

EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

contract TestSetupEvmVersion is Test {
    function setUp() public {
        evm.setEvmVersion("istanbul");
    }

    function test_evm_version_in_setup() public {
        vm.createSelectFork("<rpc>");
        // revert with NotActivated for istanbul
        ICreate2Deployer(0x35Da41c476fA5c6De066f20556069096A1F39364).computeAddress(bytes32(0), bytes32(0));
    }
}
   "#.replace("<rpc>", &endpoint),
    );
    cmd.forge_fuse()
        .args(["test", "--mc", "TestSetupEvmVersion", "-vvvv"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: EvmError: NotActivated] test_evm_version_in_setup() ([GAS])
Traces:
  [..] TestSetupEvmVersion::setUp()
    ├─ [0] VM::setEvmVersion("istanbul")
    │   └─ ← [Return]
    └─ ← [Stop]

  [..] TestSetupEvmVersion::test_evm_version_in_setup()
    └─ ← [NotActivated] EvmError: NotActivated
...

"#]]);

    // Test evm version set in constructor is accounted in test.
    prj.add_test(
        "TestConstructorEvmVersion.t.sol",
        &r#"
import {Test} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
}

interface ICreate2Deployer {
    function computeAddress(bytes32 salt, bytes32 codeHash) external view returns (address);
}

EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

contract TestConstructorEvmVersion is Test {
    constructor() {
        evm.setEvmVersion("istanbul");
    }

    function test_evm_version_in_constructor() public {
        vm.createSelectFork("<rpc>");
        // revert with NotActivated for istanbul
        ICreate2Deployer(0x35Da41c476fA5c6De066f20556069096A1F39364).computeAddress(bytes32(0), bytes32(0));
    }
}
   "#.replace("<rpc>", &endpoint),
    );
    cmd.forge_fuse()
        .args(["test", "--mc", "TestConstructorEvmVersion", "-vvvv"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: EvmError: NotActivated] test_evm_version_in_constructor() ([GAS])
Traces:
  [..] TestConstructorEvmVersion::test_evm_version_in_constructor()
    └─ ← [NotActivated] EvmError: NotActivated
...

"#]]);
});

#[cfg(feature = "monad")]
forgetest_init!(test_set_evm_version_monad_hardfork, |prj, cmd| {
    prj.add_test(
        "MonadEvmVersion.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
}

contract MonadEvmVersionTest is Test {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));
    address constant CLZ_TARGET = address(uint160(0x0c17));
    address constant MEMORY_TARGET = address(uint160(0x3e3));

    function test_set_monad_evm_version() public {
        vm.etch(CLZ_TARGET, hex"60011e60005260206000f3");

        evm.setEvmVersion("MonadEight");
        assertEq(evm.getEvmVersion(), "monadeight");
        assertEq(memoryExpansionGasDelta(), 897, "MonadEight should use Ethereum memory pricing");
        (bool ok,) = CLZ_TARGET.staticcall(hex"");
        assertFalse(ok, "CLZ should be unavailable on MonadEight");

        evm.setEvmVersion("MonadNine");
        assertEq(evm.getEvmVersion(), "monadnine");
        assertEq(memoryExpansionGasDelta(), 128, "MonadNine should use MIP-3 memory pricing");
        bytes memory output;
        (ok, output) = CLZ_TARGET.staticcall(hex"");
        assertTrue(ok, "CLZ should be available on MonadNine");
        assertEq(abi.decode(output, (uint256)), 255);

        evm.setEvmVersion("monad:MonadEight");
        assertEq(evm.getEvmVersion(), "monadeight");
        assertEq(memoryExpansionGasDelta(), 897, "MonadEight memory pricing should be restored");
        (ok,) = CLZ_TARGET.staticcall(hex"");
        assertFalse(ok, "CLZ should be disabled after switching back to MonadEight");
    }

    function memoryExpansionGasDelta() internal returns (uint256) {
        // The probe measures gas around MSTORE at offsets 0 and 0x2000.
        uint256 base = memoryGas(hex"5a5f610000525a90035f5260205ff3");
        uint256 expanded = memoryGas(hex"5a5f612000525a90035f5260205ff3");
        return expanded - base;
    }

    function memoryGas(bytes memory code) internal returns (uint256) {
        vm.etch(MEMORY_TARGET, code);
        (bool ok, bytes memory output) = MEMORY_TARGET.staticcall(hex"");
        assertTrue(ok, "memory gas probe should succeed");
        return abi.decode(output, (uint256));
    }
}
   "#,
    );

    cmd.args(["test", "--network", "monad", "--mc", "MonadEvmVersionTest"]).assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(fork_resolves_monad_hardfork_from_timestamp, |prj, cmd| {
    let activation = MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
    prj.add_test(
        "MonadForkHardfork.t.sol",
        r#"
interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
}

contract MonadForkHardforkTest {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function test_monad_eight() public {
        require(
            keccak256(bytes(evm.getEvmVersion())) == keccak256("monadeight"),
            "expected MonadEight"
        );
    }

    function test_monad_nine() public {
        require(
            keccak256(bytes(evm.getEvmVersion())) == keccak256("monadnine"),
            "expected MonadNine"
        );
    }
}
   "#,
    );

    let (_api, before) = spawn(
        NodeConfig::test().with_chain_id(Some(143u64)).with_genesis_timestamp(Some(activation - 1)),
    )
    .await;
    cmd.args([
        "test",
        "--network",
        "monad",
        "--fork-url",
        &before.http_endpoint(),
        "--mt",
        "test_monad_eight",
    ])
    .assert_success();

    let (_api, after) = spawn(
        NodeConfig::test().with_chain_id(Some(143u64)).with_genesis_timestamp(Some(activation)),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--fork-url",
            &after.http_endpoint(),
            "--mt",
            "test_monad_nine",
        ])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_init!(test_monad_memory_limit, |prj, cmd| {
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
        config.memory_limit = 128 * 1024 * 1024;
    });
    prj.add_test(
        "MonadMemoryLimit.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract MonadMemoryLimitTest is Test {
    function test_memory_ending_at_limit() public {
        uint256 value;
        assembly {
            // The stored word ends exactly at 8 MiB.
            mstore(0x7fffe0, 1)
            value := mload(0x7fffe0)
        }
        assertEq(value, 1);
    }

    function test_memory_ending_above_limit() public {
        uint256 value;
        assembly {
            // The stored word starts at 8 MiB and ends one word above the limit.
            mstore(0x800000, 1)
            value := mload(0x800000)
        }
        assertEq(value, 1);
    }
}
   "#,
    );

    cmd.args([
        "test",
        "--network",
        "monad",
        "--mc",
        "MonadMemoryLimitTest",
        "--mt",
        "test_memory_ending_at_limit",
    ])
    .assert_success();

    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--mc",
            "MonadMemoryLimitTest",
            "--mt",
            "test_memory_ending_above_limit",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: EvmError: MemoryLimitOOG] test_memory_ending_above_limit() ([GAS])
...
"#]]);
});

#[cfg(feature = "monad")]
forgetest_async!(execute_transaction_uses_monad_fork_context, |prj, cmd| {
    const CHAIN_ID: u64 = 31_337;
    const GAS_LIMIT: u64 = 100_000;
    const MAX_FEE_PER_GAS: u128 = 2_000_000_000;
    const MAX_PRIORITY_FEE_PER_GAS: u128 = 1_000_000_000;

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let ancestor = wallets[0].address();
    let control = wallets[1].address();
    let probe = Address::with_last_byte(0x20);

    // Mine the ancestor in the block Forge will fork. The synthetic transaction should execute
    // in a child of this block, making this sender ineligible to dip into its reserve.
    let mut ancestor_marker = TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: TxKind::Call(wallets[2].address()),
        value: U256::ONE,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut ancestor_marker).unwrap();
    let mut encoded = Vec::new();
    ancestor_marker.into_signed(signature).eip2718_encode(&mut encoded);
    provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();

    let value = U256::from(3_000_000_000_000_000_000u128);
    let mut ancestor_tx = TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 1,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: TxKind::Call(probe),
        value,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut ancestor_tx).unwrap();
    let mut ancestor_raw = Vec::new();
    ancestor_tx.into_signed(signature).eip2718_encode(&mut ancestor_raw);

    let mut control_tx = TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 0,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: TxKind::Call(probe),
        value,
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut control_tx).unwrap();
    let mut control_raw = Vec::new();
    control_tx.into_signed(signature).eip2718_encode(&mut control_raw);

    let source = r#"
interface Vm {
    function createSelectFork(string calldata url) external returns (uint256 forkId);
    function deal(address account, uint256 newBalance) external;
    function etch(address target, bytes calldata newRuntimeBytecode) external;
    function executeTransaction(bytes calldata rawTx) external returns (bytes memory);
}

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract ExecuteTransactionMonadContextTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    IReserveBalance constant RESERVE_BALANCE = IReserveBalance(address(0x1001));
    address constant ANCESTOR = <ancestor>;
    address constant CONTROL = <control>;
    address constant PROBE = <probe>;

    function test_execute_transaction_uses_ancestor_context() public {
        vm.createSelectFork("<rpc>");
        vm.deal(ANCESTOR, 12 ether);
        vm.deal(CONTROL, 12 ether);

        // Calls dippedIntoReserve() after receiving value, then returns the result.
        vm.etch(PROBE, hex"633a61584e5f5260205f6004601c5f6110015af15060205ff3");

        bytes memory ancestorResult = vm.executeTransaction(hex"<ancestor_raw>");
        require(abi.decode(ancestorResult, (bool)), "ancestor sender must preserve reserve");
        require(ANCESTOR.balance == 9 ether, "unexpected ancestor balance");

        // The nested transaction's tracker must not replace the outer transaction's tracker.
        require(!RESERVE_BALANCE.dippedIntoReserve(), "nested tracker leaked into parent");

        bytes memory controlResult = vm.executeTransaction(hex"<control_raw>");
        require(!abi.decode(controlResult, (bool)), "fresh sender should be allowed to dip");
        require(CONTROL.balance == 9 ether, "unexpected control balance");
    }
}
"#
    .replace("<ancestor>", &ancestor.to_string())
    .replace("<control>", &control.to_string())
    .replace("<probe>", &probe.to_string())
    .replace("<rpc>", &handle.http_endpoint())
    .replace("<ancestor_raw>", &hex::encode(ancestor_raw))
    .replace("<control_raw>", &hex::encode(control_raw));
    prj.add_test("ExecuteTransactionMonadContext.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
    });

    cmd.args(["test", "--network", "monad", "--mc", "ExecuteTransactionMonadContextTest"])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(transaction_fork_excludes_future_monad_participants, |prj, cmd| {
    const CHAIN_ID: u64 = 31_337;
    const GAS_LIMIT: u64 = 100_000;
    const MAX_FEE_PER_GAS: u128 = 3_000_000_000;

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let target_sender = wallets[3].address();
    let future_sender = wallets[0].address();
    let probe = Address::with_last_byte(0x21);
    let target_recipient = Address::with_last_byte(0x22);
    let future_recipient = Address::with_last_byte(0x23);
    let parent_block = provider.get_block_number().await.unwrap();

    let mut target_tx = TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 2_000_000_000,
        to: TxKind::Call(target_recipient),
        value: U256::ONE,
        ..Default::default()
    };
    let signature = wallets[3].sign_transaction_sync(&mut target_tx).unwrap();
    let mut target_raw = Vec::new();
    target_tx.into_signed(signature).eip2718_encode(&mut target_raw);

    let mut future_marker = TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(future_recipient),
        value: U256::ONE,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut future_marker).unwrap();
    let mut future_marker_raw = Vec::new();
    future_marker.into_signed(signature).eip2718_encode(&mut future_marker_raw);

    let mut future_probe = TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(probe),
        value: U256::from(3_000_000_000_000_000_000u128),
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut future_probe).unwrap();
    let mut future_probe_raw = Vec::new();
    future_probe.into_signed(signature).eip2718_encode(&mut future_probe_raw);

    let mut target_probe = TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 1,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(probe),
        value: U256::from(3_000_000_000_000_000_000u128),
        ..Default::default()
    };
    let signature = wallets[3].sign_transaction_sync(&mut target_probe).unwrap();
    let mut target_probe_raw = Vec::new();
    target_probe.into_signed(signature).eip2718_encode(&mut target_probe_raw);

    let mut replayed_future_probe = TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 1,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(probe),
        value: U256::from(3_000_000_000_000_000_000u128),
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut replayed_future_probe).unwrap();
    let mut replayed_future_probe_raw = Vec::new();
    replayed_future_probe.into_signed(signature).eip2718_encode(&mut replayed_future_probe_raw);

    api.anvil_set_auto_mine(false).await.unwrap();
    let target_pending = provider.send_raw_transaction(&target_raw).await.unwrap();
    let target_hash = *target_pending.tx_hash();
    let future_pending = provider.send_raw_transaction(&future_marker_raw).await.unwrap();
    let future_hash = *future_pending.tx_hash();
    api.mine_one().await;
    let target_receipt = target_pending.get_receipt().await.unwrap();
    let future_receipt = future_pending.get_receipt().await.unwrap();
    assert_eq!(target_receipt.block_number(), future_receipt.block_number());
    assert_eq!(target_receipt.block_number(), Some(parent_block + 1));
    assert_eq!(target_receipt.transaction_index(), Some(0));
    assert_eq!(future_receipt.transaction_index(), Some(1));

    let source = r#"
interface Vm {
    function createSelectFork(string calldata url) external returns (uint256 forkId);
    function createSelectFork(string calldata url, bytes32 transaction) external returns (uint256);
    function deal(address account, uint256 newBalance) external;
    function etch(address target, bytes calldata newRuntimeBytecode) external;
    function executeTransaction(bytes calldata rawTx) external returns (bytes memory);
    function getNonce(address account) external view returns (uint64 nonce);
    function rollFork(uint256 forkId, uint256 blockNumber) external;
    function transact(uint256 forkId, bytes32 txHash) external;
}

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract TransactionForkMonadContextTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant TARGET_SENDER = <target_sender>;
    address constant FUTURE_SENDER = <future_sender>;
    address constant PROBE = <probe>;
    address constant TARGET_RECIPIENT = <target_recipient>;
    address constant FUTURE_RECIPIENT = <future_recipient>;
    bytes32 constant TARGET_HASH = <target_hash>;
    bytes32 constant FUTURE_HASH = <future_hash>;
    uint256 constant PARENT_BLOCK = <parent_block>;

    function test_future_transaction_is_not_an_ancestor() public {
        vm.createSelectFork("<rpc>", TARGET_HASH);
        vm.deal(FUTURE_SENDER, 12 ether);

        // Calls dippedIntoReserve() after receiving value, then returns the result.
        vm.etch(PROBE, hex"633a61584e5f5260205f6004601c5f6110015af15060205ff3");
        bytes memory result = vm.executeTransaction(hex"<future_probe_raw>");
        require(!abi.decode(result, (bool)), "future sender must be allowed to dip");
        require(FUTURE_SENDER.balance == 9 ether, "unexpected future sender balance");
    }

    function test_parent_block_transact_advances_to_next_transaction() public {
        uint256 forkId = vm.createSelectFork("<rpc>");
        vm.rollFork(forkId, PARENT_BLOCK);
        uint64 targetNonce = vm.getNonce(TARGET_SENDER);
        uint256 targetRecipientBalance = TARGET_RECIPIENT.balance;
        uint256 futureRecipientBalance = FUTURE_RECIPIENT.balance;

        vm.transact(forkId, TARGET_HASH);

        require(vm.getNonce(TARGET_SENDER) == targetNonce + 1, "target nonce was not advanced");
        require(TARGET_RECIPIENT.balance == targetRecipientBalance + 1, "target was not committed");
        require(FUTURE_RECIPIENT.balance == futureRecipientBalance, "future tx was committed");

        vm.deal(TARGET_SENDER, 12 ether);
        vm.deal(FUTURE_SENDER, 12 ether);
        vm.etch(PROBE, hex"633a61584e5f5260205f6004601c5f6110015af15060205ff3");

        bytes memory targetResult = vm.executeTransaction(hex"<target_probe_raw>");
        require(abi.decode(targetResult, (bool)), "replayed sender was treated as fresh");
        require(TARGET_SENDER.balance == 9 ether, "unexpected replayed sender balance");

        bytes memory futureResult = vm.executeTransaction(hex"<future_probe_raw>");
        require(!abi.decode(futureResult, (bool)), "future sender became an ancestor");
        require(FUTURE_SENDER.balance == 9 ether, "unexpected future sender balance");
    }

    function test_sequential_transacts_advance_past_block() public {
        uint256 forkId = vm.createSelectFork("<rpc>");
        vm.rollFork(forkId, PARENT_BLOCK);
        uint64 targetNonce = vm.getNonce(TARGET_SENDER);
        uint64 futureNonce = vm.getNonce(FUTURE_SENDER);
        uint256 targetRecipientBalance = TARGET_RECIPIENT.balance;
        uint256 futureRecipientBalance = FUTURE_RECIPIENT.balance;

        vm.transact(forkId, TARGET_HASH);
        vm.transact(forkId, FUTURE_HASH);

        require(vm.getNonce(TARGET_SENDER) == targetNonce + 1, "target nonce was not advanced");
        require(vm.getNonce(FUTURE_SENDER) == futureNonce + 1, "future nonce was not advanced");
        require(TARGET_RECIPIENT.balance == targetRecipientBalance + 1, "target was not committed");
        require(FUTURE_RECIPIENT.balance == futureRecipientBalance + 1, "future was not committed");

        vm.deal(FUTURE_SENDER, 12 ether);
        vm.etch(PROBE, hex"633a61584e5f5260205f6004601c5f6110015af15060205ff3");
        bytes memory result = vm.executeTransaction(hex"<replayed_future_probe_raw>");
        require(abi.decode(result, (bool)), "last replayed sender was treated as fresh");
        require(FUTURE_SENDER.balance == 9 ether, "unexpected last sender balance");
    }
}
"#
    .replace("<target_sender>", &target_sender.to_string())
    .replace("<future_sender>", &future_sender.to_string())
    .replace("<probe>", &probe.to_string())
    .replace("<target_recipient>", &target_recipient.to_string())
    .replace("<future_recipient>", &future_recipient.to_string())
    .replace("<rpc>", &handle.http_endpoint())
    .replace("<target_hash>", &target_hash.to_string())
    .replace("<future_hash>", &future_hash.to_string())
    .replace("<parent_block>", &parent_block.to_string())
    .replace("<future_probe_raw>", &hex::encode(future_probe_raw))
    .replace("<target_probe_raw>", &hex::encode(target_probe_raw))
    .replace("<replayed_future_probe_raw>", &hex::encode(replayed_future_probe_raw));
    prj.add_test("TransactionForkMonadContext.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
    });

    cmd.args(["test", "--network", "monad", "--mc", "TransactionForkMonadContextTest"])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(transact_replays_monad_protocol_system_target_forks, |prj, cmd| {
    const SYSTEM_ADDRESS: Address = address!("0x6f49a8F621353f12378d0046E7d7e4b9B249DC9e");
    const STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");
    const BLOCK_AUTHOR: Address = address!("0x1111111111111111111111111111111111111111");
    const VALIDATOR_AUTH: Address = address!("0x2222222222222222222222222222222222222222");
    const VALIDATOR_ID: u64 = 7;

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let mon = U256::from(1_000_000_000_000_000_000u128);
    let reward = U256::from(25) * mon;
    let initial_system_balance = U256::from(100) * mon;
    let initial_staking_balance = U256::from(3) * mon;

    api.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();
    api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(11)).await.unwrap();
    api.anvil_set_balance(SYSTEM_ADDRESS, initial_system_balance).await.unwrap();
    api.anvil_set_balance(STAKING_ADDRESS, initial_staking_balance).await.unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        monad_staking_validator_id_key(BLOCK_AUTHOR),
        storage_value(left_aligned_u64(VALIDATOR_ID)),
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        monad_staking_validator_key(0x04, VALIDATOR_ID, 0),
        storage_value(U256::from(100) * mon),
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        monad_staking_validator_key(0x04, VALIDATOR_ID, 1),
        B256::ZERO,
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        monad_staking_validator_key(0x09, VALIDATOR_ID, 6),
        storage_value(address_and_flags(VALIDATOR_AUTH, 0)),
    )
    .await
    .unwrap();

    api.mine_one().await;
    let parent_block = provider.get_block_number().await.unwrap();

    let mut reward_input = keccak256("syscallReward(address)")[..4].to_vec();
    reward_input.extend_from_slice(&[0u8; 12]);
    reward_input.extend_from_slice(BLOCK_AUTHOR.as_slice());

    let request = <Ethereum as Network>::TransactionRequest::default()
        .with_from(SYSTEM_ADDRESS)
        .with_to(STAKING_ADDRESS)
        .with_value(reward)
        .with_input(reward_input)
        .with_gas_limit(1_000_000)
        .with_gas_price(2_000_000_000);
    let receipt =
        provider.send_transaction(request.into()).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
    assert_eq!(receipt.block_number(), Some(parent_block + 1));

    let target_hash = receipt.transaction_hash;
    let endpoint = spawn_canonical_monad_reward_rpc(handle.http_endpoint(), target_hash).await;
    let transaction =
        rpc_request(&endpoint, "eth_getTransactionByHash", json!([target_hash])).await;
    assert_eq!(transaction["result"]["gas"], "0x0");
    assert_eq!(transaction["result"]["gasPrice"], "0x0");
    assert_eq!(transaction["result"]["r"], "0x0");
    assert_eq!(transaction["result"]["s"], "0x0");
    assert_eq!(transaction["result"]["type"], "0x0");
    assert_eq!(transaction["result"]["v"], "0x0");
    assert_eq!(transaction["result"]["value"], format!("{reward:#x}"));

    let canonical_receipt =
        rpc_request(&endpoint, "eth_getTransactionReceipt", json!([target_hash])).await;
    assert_eq!(canonical_receipt["result"]["status"], "0x1");
    assert_eq!(canonical_receipt["result"]["gasUsed"], "0x0");
    assert_eq!(canonical_receipt["result"]["effectiveGasPrice"], "0x0");

    let target_block = rpc_request(
        &endpoint,
        "eth_getBlockByNumber",
        json!([format!("{:#x}", parent_block + 1), true]),
    )
    .await;
    assert_ne!(target_block["result"]["baseFeePerGas"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["hash"], target_hash.to_string());
    assert_eq!(target_block["result"]["transactions"][0]["gas"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["gasPrice"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["r"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["s"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["v"], "0x0");

    let source = r#"
interface Vm {
    struct Log {
        bytes32[] topics;
        bytes data;
        address emitter;
    }

    function createSelectFork(string calldata url, bytes32 txHash) external returns (uint256 forkId);
    function createSelectFork(string calldata url, uint256 blockNumber) external returns (uint256 forkId);
    function recordLogs() external;
    function getRecordedLogs() external returns (Log[] memory entries);
    function getNonce(address account) external view returns (uint64 nonce);
    function transact(bytes32 txHash) external;
}

interface IMonadStaking {
    function getProposerValId() external returns (uint64 validatorId);
    function getValidator(uint64 validatorId) external;
}

contract MonadProtocolSystemTargetTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant SYSTEM = 0x6f49a8F621353f12378d0046E7d7e4b9B249DC9e;
    IMonadStaking constant STAKING = IMonadStaking(address(0x1000));
    bytes32 constant TARGET_HASH = <tx_hash>;
    uint256 constant PARENT_BLOCK = <parent_block>;
    uint64 constant VALIDATOR_ID = 7;
    uint256 constant REWARD = 25 ether;

    function test_reward_target_from_transaction_hash_fork() public {
        vm.createSelectFork("<rpc>", TARGET_HASH);
        _assertRewardReplay();
    }

    function test_reward_target_from_parent_block_fork() public {
        vm.createSelectFork("<rpc>", PARENT_BLOCK);
        _assertRewardReplay();
    }

    function test_gas_paying_system_sender_target_fork_is_rejected() public {
        vm.createSelectFork("<origin_rpc>", TARGET_HASH);
        uint256 systemBalanceBefore = SYSTEM.balance;
        uint256 stakingBalanceBefore = address(STAKING).balance;
        uint64 nonceBefore = vm.getNonce(SYSTEM);

        bool reverted;
        try vm.transact(TARGET_HASH) {
            reverted = false;
        } catch {
            reverted = true;
        }

        require(reverted, "noncanonical system envelope was replayed");
        require(SYSTEM.balance == systemBalanceBefore, "protocol caller balance changed");
        require(address(STAKING).balance == stakingBalanceBefore, "staking balance changed");
        require(vm.getNonce(SYSTEM) == nonceBefore, "protocol caller nonce changed");
    }

    function _assertRewardReplay() internal {
        uint256 systemBalanceBefore = SYSTEM.balance;
        uint256 stakingBalanceBefore = address(STAKING).balance;
        uint64 nonceBefore = vm.getNonce(SYSTEM);
        (uint256 accumulatorBefore, uint256 unclaimedBefore) = _validatorRewards();
        require(nonceBefore == 11, "unexpected protocol caller nonce");
        require(stakingBalanceBefore == 3 ether, "unexpected staking prestate balance");
        require(accumulatorBefore == 0, "unexpected reward accumulator");
        require(unclaimedBefore == 0, "unexpected unclaimed rewards");

        vm.recordLogs();
        vm.transact(TARGET_HASH);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        require(SYSTEM.balance == systemBalanceBefore, "protocol caller paid gas or value");
        require(vm.getNonce(SYSTEM) == nonceBefore + 1, "protocol caller nonce was not advanced");
        require(address(STAKING).balance == stakingBalanceBefore + REWARD, "reward was not minted");
        require(STAKING.getProposerValId() == VALIDATOR_ID, "proposer validator was not updated");

        (uint256 accumulatorAfter, uint256 unclaimedAfter) = _validatorRewards();
        require(accumulatorAfter > accumulatorBefore, "reward accumulator was not updated");
        require(unclaimedAfter == unclaimedBefore + REWARD, "validator reward was not credited");

        require(logs.length == 1, "unexpected reward log count");
        require(logs[0].emitter == address(STAKING), "unexpected reward log emitter");
        require(logs[0].topics.length == 3, "unexpected reward log topics");
        require(
            logs[0].topics[0] == keccak256("ValidatorRewarded(uint64,address,uint256,uint64)"),
            "unexpected reward event"
        );
        require(uint256(logs[0].topics[1]) == uint256(VALIDATOR_ID), "unexpected validator topic");
        require(
            logs[0].topics[2] == bytes32(uint256(uint160(SYSTEM))),
            "unexpected reward sender topic"
        );
        (uint256 amount, uint64 epoch) = abi.decode(logs[0].data, (uint256, uint64));
        require(amount == REWARD, "unexpected logged reward");
        require(epoch == 0, "unexpected reward epoch");
    }

    function _validatorRewards() internal returns (uint256 accumulator, uint256 unclaimed) {
        (bool ok, bytes memory result) = address(STAKING).call(
            abi.encodeWithSelector(IMonadStaking.getValidator.selector, VALIDATOR_ID)
        );
        require(ok && result.length >= 192, "failed to read validator");
        assembly {
            accumulator := mload(add(result, 128))
            unclaimed := mload(add(result, 192))
        }
    }
}
"#
    .replace("<rpc>", &endpoint)
    .replace("<origin_rpc>", &handle.http_endpoint())
    .replace("<tx_hash>", &target_hash.to_string())
    .replace("<parent_block>", &parent_block.to_string());
    prj.add_test("MonadProtocolSystemTarget.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
    });

    cmd.args(["test", "--network", "monad", "--mc", "MonadProtocolSystemTargetTest"])
        .assert_success();
});

forgetest_init!(test_set_evm_version_tempo_hardfork, |prj, cmd| {
    prj.update_config(|config| {
        config.solc = Some(OTHER_SOLC_VERSION.into());
    });

    prj.add_test(
        "TempoEvmVersion.t.sol",
        r#"
pragma solidity >=0.8.20;

import {Test} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
}

contract TempoEvmVersionTest is Test {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function test_set_tempo_evm_version() public {
        evm.setEvmVersion("T3");
        assertEq(evm.getEvmVersion(), "t3");

        evm.setEvmVersion("tempo:T2");
        assertEq(evm.getEvmVersion(), "t2");
    }
}
   "#,
    );

    cmd.args(["test", "--network", "tempo", "--mc", "TempoEvmVersionTest"]).assert_success();
});

forgetest_init!(test_network_tempo_defaults_to_latest_hardfork, |prj, cmd| {
    prj.update_config(|config| {
        config.solc = Some(OTHER_SOLC_VERSION.into());
    });

    let expected =
        foundry_evm::hardforks::latest_active_tempo_hardfork().to_string().to_lowercase();
    prj.add_test(
        "TempoDefaultEvmVersion.t.sol",
        &format!(
            r#"
pragma solidity >=0.8.20;

import {{Test}} from "forge-std/Test.sol";

interface EvmVm {{
    function getEvmVersion() external pure returns (string memory evm);
}}

contract TempoDefaultEvmVersionTest is Test {{
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function test_network_tempo_defaults_to_latest_hardfork() public {{
        assertEq(evm.getEvmVersion(), "{expected}");
    }}
}}
   "#
        ),
    );

    cmd.args(["test", "--network", "tempo", "--mc", "TempoDefaultEvmVersionTest"]).assert_success();
});

// Validates T5 implicit-approval wiring: the cheatcodes, the AddressRegistry selector,
// unchanged standard approve/transferFrom behavior, an implicit pull through StablecoinDEX,
// and a non-implicit spender control case.
forgetest_init!(test_tempo_implicit_approval_t5, |prj, cmd| {
    prj.update_config(|config| {
        config.solc = Some(OTHER_SOLC_VERSION.into());
        // The precompile registry snapshots `cfg.spec` at EVM construction, so pinning T5
        // here is what activates the T5 precompiles and selectors. `vm.setEvmVersion` only
        // flips the cheatcode-visible spec.
        config.hardfork = Some(FoundryHardfork::Tempo(TempoHardfork::T5));
    });

    let fixture = include_str!("../../fixtures/TempoImplicitApproval.t.sol");
    prj.add_test("TempoImplicitApproval.t.sol", fixture);

    cmd.args(["test", "--network", "tempo", "--mc", "TempoImplicitApprovalTest"]).assert_success();
});

// Regression test for <https://github.com/foundry-rs/foundry/issues/13040>:
// configured evm_version must be preserved after createSelectFork / rollFork.
forgetest_init!(test_fork_preserves_evm_version, |prj, cmd| {
    let endpoint = rpc::next_http_archive_rpc_url();

    prj.update_config(|config| {
        config.evm_version = EvmVersion::Cancun;
    });

    prj.add_test(
        "ForkEvmVersion.t.sol",
        &r#"
import {Test} from "forge-std/Test.sol";

contract ForkEvmVersionTest is Test {
    function test_evm_version_preserved_after_fork() public {
        assertEq(vm.getEvmVersion(), "cancun", "before fork");
        uint256 forkId = vm.createSelectFork("<rpc>", 21000000);
        assertEq(vm.getEvmVersion(), "cancun", "after createSelectFork");
        vm.rollFork(21000001);
        assertEq(vm.getEvmVersion(), "cancun", "after rollFork");
    }
}
   "#
        .replace("<rpc>", &endpoint),
    );

    cmd.args(["test", "--mc", "ForkEvmVersionTest", "-vvvv"]).assert_success();
});
