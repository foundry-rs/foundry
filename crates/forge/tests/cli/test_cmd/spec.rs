#[cfg(feature = "monad")]
use alloy_consensus::{SignableTransaction, TxEip1559};
#[cfg(feature = "monad")]
use alloy_network::{Ethereum, Network, ReceiptResponse, TransactionBuilder, TxSignerSync};
#[cfg(feature = "monad")]
use alloy_primitives::{Address, TxKind, U256, address, hex, keccak256};
#[cfg(feature = "monad")]
use alloy_provider::Provider;
#[cfg(feature = "monad")]
use anvil::{NodeConfig, spawn};
use foundry_compilers::artifacts::EvmVersion;
use foundry_evm::hardforks::{FoundryHardfork, TempoHardfork};
use foundry_test_utils::{rpc, util::OTHER_SOLC_VERSION};

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
    let future_sender = wallets[0].address();
    let probe = Address::with_last_byte(0x21);

    let mut target_tx = TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 2_000_000_000,
        to: TxKind::Call(wallets[4].address()),
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
        to: TxKind::Call(wallets[5].address()),
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

    api.anvil_set_auto_mine(false).await.unwrap();
    let target_pending = provider.send_raw_transaction(&target_raw).await.unwrap();
    let target_hash = *target_pending.tx_hash();
    let future_pending = provider.send_raw_transaction(&future_marker_raw).await.unwrap();
    api.mine_one().await;
    let target_receipt = target_pending.get_receipt().await.unwrap();
    let future_receipt = future_pending.get_receipt().await.unwrap();
    assert_eq!(target_receipt.block_number(), future_receipt.block_number());
    assert!(target_receipt.transaction_index() < future_receipt.transaction_index());

    let source = r#"
interface Vm {
    function createSelectFork(string calldata url, bytes32 transaction) external returns (uint256);
    function deal(address account, uint256 newBalance) external;
    function etch(address target, bytes calldata newRuntimeBytecode) external;
    function executeTransaction(bytes calldata rawTx) external returns (bytes memory);
}

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract TransactionForkMonadContextTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant FUTURE_SENDER = <future_sender>;
    address constant PROBE = <probe>;

    function test_future_transaction_is_not_an_ancestor() public {
        vm.createSelectFork("<rpc>", <target_hash>);
        vm.deal(FUTURE_SENDER, 12 ether);

        // Calls dippedIntoReserve() after receiving value, then returns the result.
        vm.etch(PROBE, hex"633a61584e5f5260205f6004601c5f6110015af15060205ff3");
        bytes memory result = vm.executeTransaction(hex"<future_probe_raw>");
        require(!abi.decode(result, (bool)), "future sender must be allowed to dip");
        require(FUTURE_SENDER.balance == 9 ether, "unexpected future sender balance");
    }
}
"#
    .replace("<future_sender>", &future_sender.to_string())
    .replace("<probe>", &probe.to_string())
    .replace("<rpc>", &handle.http_endpoint())
    .replace("<target_hash>", &target_hash.to_string())
    .replace("<future_probe_raw>", &hex::encode(future_probe_raw));
    prj.add_test("TransactionForkMonadContext.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
    });

    cmd.args(["test", "--network", "monad", "--mc", "TransactionForkMonadContextTest"])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(transact_replays_monad_protocol_system_target, |prj, cmd| {
    const SYSTEM_ADDRESS: Address = address!("0x6f49a8F621353f12378d0046E7d7e4b9B249DC9e");
    const STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");

    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let initial_balance = U256::from(1_000_000_000_000_000_000u128);
    api.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();
    api.anvil_set_balance(SYSTEM_ADDRESS, initial_balance).await.unwrap();

    let request = <Ethereum as Network>::TransactionRequest::default()
        .with_from(SYSTEM_ADDRESS)
        .with_to(STAKING_ADDRESS)
        .with_input(keccak256("syscallSnapshot()")[..4].to_vec())
        .with_gas_limit(1_000_000);
    let receipt =
        provider.send_transaction(request.into()).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
    let source = r#"
interface Vm {
    function createSelectFork(string calldata url, bytes32 txHash) external returns (uint256 forkId);
    function getNonce(address account) external view returns (uint64 nonce);
    function transact(bytes32 txHash) external;
}

contract MonadProtocolSystemTargetTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant SYSTEM = 0x6f49a8F621353f12378d0046E7d7e4b9B249DC9e;

    function test_transact_uses_protocol_system_path() public {
        vm.createSelectFork("<rpc>", <tx_hash>);
        uint256 balanceBefore = SYSTEM.balance;
        uint64 nonceBefore = vm.getNonce(SYSTEM);

        vm.transact(<tx_hash>);

        require(SYSTEM.balance == balanceBefore, "protocol caller paid ordinary transaction gas");
        require(vm.getNonce(SYSTEM) == nonceBefore + 1, "protocol caller nonce was not advanced");
    }
}
"#
    .replace("<rpc>", &handle.http_endpoint())
    .replace("<tx_hash>", &receipt.transaction_hash.to_string());
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
