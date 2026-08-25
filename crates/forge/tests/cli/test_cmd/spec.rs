use foundry_compilers::artifacts::EvmVersion;
use foundry_evm::hardforks::{FoundryHardfork, TempoHardfork};
use foundry_test_utils::{rpc, util::OTHER_SOLC_VERSION};

#[cfg(feature = "monad")]
async fn rpc_request(endpoint: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    reqwest::Client::new()
        .post(endpoint)
        .json(&serde_json::json!({
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
fn monad_staking_reward_input(block_author: alloy_primitives::Address) -> Vec<u8> {
    let mut input = alloy_primitives::keccak256("syscallReward(address)")[..4].to_vec();
    input.extend_from_slice(&[0u8; 12]);
    input.extend_from_slice(block_author.as_slice());
    input
}

#[cfg(feature = "monad")]
fn monad_staking_validator_id_key(address: alloy_primitives::Address) -> alloy_primitives::U256 {
    let mut key = [0u8; 32];
    key[0] = 0x06;
    key[1..21].copy_from_slice(address.as_slice());
    alloy_primitives::U256::from_be_bytes(key)
}

#[cfg(feature = "monad")]
fn monad_staking_validator_key(
    namespace: u8,
    validator_id: u64,
    offset: u8,
) -> alloy_primitives::U256 {
    let mut key = [0u8; 32];
    key[0] = namespace;
    key[1..9].copy_from_slice(&validator_id.to_be_bytes());
    alloy_primitives::U256::from_be_bytes(key) + alloy_primitives::U256::from(offset)
}

#[cfg(feature = "monad")]
fn left_aligned_u64(value: u64) -> alloy_primitives::U256 {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&value.to_be_bytes());
    alloy_primitives::U256::from_be_bytes(bytes)
}

#[cfg(feature = "monad")]
fn address_and_flags(address: alloy_primitives::Address, flags: u64) -> alloy_primitives::U256 {
    let mut bytes = [0u8; 32];
    bytes[..20].copy_from_slice(address.as_slice());
    bytes[20..28].copy_from_slice(&flags.to_be_bytes());
    alloy_primitives::U256::from_be_bytes(bytes)
}

#[cfg(feature = "monad")]
fn storage_value(value: alloy_primitives::U256) -> alloy_primitives::B256 {
    alloy_primitives::B256::from(value.to_be_bytes::<32>())
}

#[cfg(feature = "monad")]
fn override_rpc_transaction_chain_id(value: &mut serde_json::Value, target: &str, chain_id: &str) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                override_rpc_transaction_chain_id(value, target, chain_id);
            }
        }
        serde_json::Value::Object(object) => {
            if object.get("hash").and_then(serde_json::Value::as_str) == Some(target) {
                object
                    .insert("chainId".to_string(), serde_json::Value::String(chain_id.to_string()));
            }
            for value in object.values_mut() {
                override_rpc_transaction_chain_id(value, target, chain_id);
            }
        }
        _ => {}
    }
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

contract ModexpGasProbe {
    function measure() external view returns (uint256) {
        bytes memory input = abi.encodePacked(
            bytes32(uint256(32)),
            bytes32(uint256(32)),
            bytes32(uint256(32)),
            bytes32(type(uint256).max),
            bytes32(type(uint256).max),
            bytes32(type(uint256).max)
        );
        uint256 gasBefore = gasleft();
        (bool ok,) = address(5).staticcall(input);
        uint256 gasUsed = gasBefore - gasleft();
        require(ok, "MODEXP probe should succeed");
        return gasUsed;
    }
}

contract StorageGasProbe {
    function measureRead(uint256 secondSlot)
        external
        view
        returns (uint256 gasUsed, uint256 checksum)
    {
        assembly {
            let gasBefore := gas()
            let first := sload(0)
            let second := sload(secondSlot)
            gasUsed := sub(gasBefore, gas())
            checksum := add(first, second)
        }
    }

    function measureWrite(uint256 secondSlot) external returns (uint256 gasUsed) {
        assembly {
            let gasBefore := gas()
            sstore(0, 1)
            sstore(secondSlot, 1)
            gasUsed := sub(gasBefore, gas())
        }
    }
}

contract MonadEvmVersionTest is Test {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));
    address constant CLZ_TARGET = address(uint160(0x0c17));
    address constant MEMORY_TARGET = address(uint160(0x3e3));
    address constant RESERVE_TARGET = address(uint160(0x1001));
    ModexpGasProbe immutable modexpGasProbe = new ModexpGasProbe();

    function test_set_monad_evm_version() public {
        assertEq(evm.getEvmVersion(), "monadten");
        assertMip8Active();

        evm.setEvmVersion("MonadEight");
        vm.etch(CLZ_TARGET, hex"60011e60005260206000f3");

        assertEq(evm.getEvmVersion(), "monadeight");
        assertEq(memoryExpansionGasDelta(), 897, "MonadEight should use Ethereum memory pricing");
        uint256 monadEightModexpGas = modexpGasProbe.measure();
        assertFalse(reservePrecompileActive(), "MonadEight should not expose the reserve precompile");
        (bool ok,) = CLZ_TARGET.staticcall(hex"");
        assertFalse(ok, "CLZ should be unavailable on MonadEight");

        evm.setEvmVersion("MonadNine");
        assertEq(evm.getEvmVersion(), "monadnine");
        assertEq(memoryExpansionGasDelta(), 128, "MonadNine should use MIP-3 memory pricing");
        uint256 monadNineModexpGas = modexpGasProbe.measure();
        assertGt(monadNineModexpGas, monadEightModexpGas, "MonadNine should use MIP-3 MODEXP pricing");
        assertTrue(reservePrecompileActive(), "MonadNine should expose the reserve precompile");
        bytes memory output;
        (ok, output) = CLZ_TARGET.staticcall(hex"");
        assertTrue(ok, "CLZ should be available on MonadNine");
        assertEq(abi.decode(output, (uint256)), 255);
        assertMip8Inactive();

        evm.setEvmVersion("monad:MonadTen");
        assertEq(evm.getEvmVersion(), "monadten");
        assertMip8Active();

        evm.setEvmVersion("monad:MonadEight");
        assertEq(evm.getEvmVersion(), "monadeight");
        assertEq(memoryExpansionGasDelta(), 897, "MonadEight memory pricing should be restored");
        assertEq(
            modexpGasProbe.measure(),
            monadEightModexpGas,
            "MonadEight MODEXP pricing should be restored exactly"
        );
        assertFalse(
            reservePrecompileActive(), "MonadEight should remove the reserve precompile again"
        );
        (ok,) = CLZ_TARGET.staticcall(hex"");
        assertFalse(ok, "CLZ should be disabled after switching back to MonadEight");
    }

    function assertMip8Active() internal {
        assertEq(storageReadGas(128) - storageReadGas(127), 8_000, "MIP-8 page read cost");
        assertEq(storageWriteGas(128) - storageWriteGas(1), 10_800, "MIP-8 page write cost");
    }

    function assertMip8Inactive() internal {
        assertEq(storageReadGas(128), storageReadGas(127), "legacy slot read cost");
        assertEq(storageWriteGas(128), storageWriteGas(1), "legacy slot write cost");
    }

    function storageReadGas(uint256 secondSlot) internal returns (uint256 gasUsed) {
        uint256 checksum;
        (gasUsed, checksum) = new StorageGasProbe().measureRead(secondSlot);
        assertEq(checksum, 0);
    }

    function storageWriteGas(uint256 secondSlot) internal returns (uint256) {
        return new StorageGasProbe().measureWrite(secondSlot);
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

    function reservePrecompileActive() internal returns (bool) {
        (bool ok, bytes memory output) = RESERVE_TARGET.call(hex"3a61584e");
        assertTrue(ok, "reserve probe should succeed");
        if (output.length == 0) return false;
        assertEq(output.length, 32, "reserve probe returned malformed output");
        assertFalse(abi.decode(output, (bool)), "fresh execution should not dip into reserve");
        return true;
    }
}
   "#,
    );

    cmd.args(["test", "--network", "monad", "--mc", "MonadEvmVersionTest"]).assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(fork_resolves_monad_hardfork_from_timestamp, |prj, cmd| {
    let monad_nine_activation =
        foundry_evm::hardforks::MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
    let mainnet_activation =
        foundry_evm::hardforks::MonadHardfork::MonadTen.mainnet_activation_timestamp().unwrap();
    let testnet_activation =
        foundry_evm::hardforks::MonadHardfork::MonadTen.testnet_activation_timestamp().unwrap();
    prj.add_test(
        "MonadForkHardfork.t.sol",
        r#"
interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
}

contract MonadForkHardforkTest {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function test_monad_eight() public {
        require(block.chainid == 1, "expected CHAINID override");
        require(
            keccak256(bytes(evm.getEvmVersion())) == keccak256("monadeight"),
            "expected MonadEight"
        );
    }

    function test_monad_nine() public {
        require(block.chainid == 1, "expected CHAINID override");
        require(
            keccak256(bytes(evm.getEvmVersion())) == keccak256("monadnine"),
            "expected MonadNine"
        );
    }

    function test_monad_ten() public {
        require(block.chainid == 1, "expected CHAINID override");
        require(
            keccak256(bytes(evm.getEvmVersion())) == keccak256("monadten"),
            "expected MonadTen"
        );
    }
}
   "#,
    );

    let (_api, before) = anvil::spawn(
        anvil::NodeConfig::test()
            .with_chain_id(Some(143u64))
            .with_genesis_timestamp(Some(monad_nine_activation - 1)),
    )
    .await;
    cmd.args([
        "test",
        "--network",
        "monad",
        "--fork-url",
        &before.http_endpoint(),
        "--chain-id",
        "1",
        "--mt",
        "test_monad_eight",
    ])
    .assert_success();

    let (_api, after) = anvil::spawn(
        anvil::NodeConfig::test()
            .with_chain_id(Some(143u64))
            .with_genesis_timestamp(Some(monad_nine_activation)),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--fork-url",
            &after.http_endpoint(),
            "--chain-id",
            "1",
            "--mt",
            "test_monad_nine",
        ])
        .assert_success();

    let (_api, before) = anvil::spawn(
        anvil::NodeConfig::test()
            .with_chain_id(Some(143u64))
            .with_genesis_timestamp(Some(mainnet_activation - 1)),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--fork-url",
            &before.http_endpoint(),
            "--chain-id",
            "1",
            "--mt",
            "test_monad_nine",
        ])
        .assert_success();

    let (_api, after) = anvil::spawn(
        anvil::NodeConfig::test()
            .with_chain_id(Some(143u64))
            .with_genesis_timestamp(Some(mainnet_activation)),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--fork-url",
            &after.http_endpoint(),
            "--chain-id",
            "1",
            "--mt",
            "test_monad_ten",
        ])
        .assert_success();

    let (_api, before) = anvil::spawn(
        anvil::NodeConfig::test()
            .with_chain_id(Some(10143u64))
            .with_genesis_timestamp(Some(testnet_activation - 1)),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--fork-url",
            &before.http_endpoint(),
            "--chain-id",
            "1",
            "--mt",
            "test_monad_nine",
        ])
        .assert_success();

    let (_api, after) = anvil::spawn(
        anvil::NodeConfig::test()
            .with_chain_id(Some(10143u64))
            .with_genesis_timestamp(Some(testnet_activation)),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--fork-url",
            &after.http_endpoint(),
            "--chain-id",
            "1",
            "--mt",
            "test_monad_ten",
        ])
        .assert_success();

    let (_api, overridden) = anvil::spawn(
        anvil::NodeConfig::test_monad()
            .with_chain_id(Some(143u64))
            .with_genesis_timestamp(Some(mainnet_activation))
            .with_hardfork(Some(foundry_evm::hardforks::MonadHardfork::MonadNine.into())),
    )
    .await;
    cmd.forge_fuse()
        .args([
            "test",
            "--fork-url",
            &overridden.http_endpoint(),
            "--chain-id",
            "1",
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
    use alloy_consensus::SignableTransaction as _;
    use alloy_network::TxSignerSync as _;
    use alloy_provider::Provider as _;

    const CHAIN_ID: u64 = 31_337;
    const GAS_LIMIT: u64 = 100_000;
    const MAX_FEE_PER_GAS: u128 = 2_000_000_000;
    const MAX_PRIORITY_FEE_PER_GAS: u128 = 1_000_000_000;

    let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let ancestor = wallets[0].address();
    let control = wallets[1].address();
    let tracked = wallets[3].address();
    let nested_only = wallets[5].address();
    let probe = alloy_primitives::Address::with_last_byte(0x20);
    let receiver = alloy_primitives::Address::with_last_byte(0x21);

    // Mine the ancestor in the block Forge will fork. The synthetic transaction should execute
    // in a child of this block, making this sender ineligible to dip into its reserve.
    let mut ancestor_marker = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: alloy_primitives::TxKind::Call(wallets[2].address()),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut ancestor_marker).unwrap();
    let mut encoded = Vec::new();
    ancestor_marker.into_signed(signature).eip2718_encode(&mut encoded);
    provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();

    let value = alloy_primitives::U256::from(3_000_000_000_000_000_000u128);
    let mut ancestor_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 1,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: alloy_primitives::TxKind::Call(probe),
        value,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut ancestor_tx).unwrap();
    let mut ancestor_raw = Vec::new();
    ancestor_tx.into_signed(signature).eip2718_encode(&mut ancestor_raw);

    let mut control_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 0,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: alloy_primitives::TxKind::Call(probe),
        value,
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut control_tx).unwrap();
    let mut control_raw = Vec::new();
    control_tx.into_signed(signature).eip2718_encode(&mut control_raw);

    let mut credit_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 0,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: alloy_primitives::TxKind::Call(tracked),
        value,
        ..Default::default()
    };
    let signature = wallets[2].sign_transaction_sync(&mut credit_tx).unwrap();
    let mut credit_raw = Vec::new();
    credit_tx.into_signed(signature).eip2718_encode(&mut credit_raw);

    let mut preserve_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 0,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: alloy_primitives::TxKind::Call(receiver),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[4].sign_transaction_sync(&mut preserve_tx).unwrap();
    let mut preserve_raw = Vec::new();
    preserve_tx.into_signed(signature).eip2718_encode(&mut preserve_raw);

    let nested_only_balance = provider.get_balance(nested_only).await.unwrap();
    let nested_only_remaining = alloy_primitives::U256::from(7_000_000_000_000_000_000u128);
    let mut nested_only_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 0,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        to: alloy_primitives::TxKind::Call(receiver),
        value: nested_only_balance - nested_only_remaining,
        ..Default::default()
    };
    let signature = wallets[5].sign_transaction_sync(&mut nested_only_tx).unwrap();
    let mut nested_only_raw = Vec::new();
    nested_only_tx.into_signed(signature).eip2718_encode(&mut nested_only_raw);

    let source = r#"
interface Vm {
    function createSelectFork(string calldata url) external returns (uint256 forkId);
    function deal(address account, uint256 newBalance) external;
    function etch(address target, bytes calldata newRuntimeBytecode) external;
    function executeTransaction(bytes calldata rawTx) external returns (bytes memory);
    function prank(address msgSender) external;
}

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract ExecuteTransactionMonadContextTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    IReserveBalance constant RESERVE_BALANCE = IReserveBalance(address(0x1001));
    address constant ANCESTOR = <ancestor>;
    address constant CONTROL = <control>;
    address constant TRACKED = <tracked>;
    address constant NESTED_ONLY = <nested_only>;
    address constant PROBE = <probe>;
    address constant RECEIVER = <receiver>;

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

    function test_execute_transaction_credit_clears_outer_violation() public {
        vm.createSelectFork("<rpc>");
        violateReserve(TRACKED);

        vm.executeTransaction(hex"<credit_raw>");

        require(TRACKED.balance == 12 ether, "unexpected credited balance");
        require(!RESERVE_BALANCE.dippedIntoReserve(), "credited violation was not cleared");
    }

    function test_execute_transaction_preserves_unaffected_outer_violation() public {
        vm.createSelectFork("<rpc>");
        violateReserve(TRACKED);

        vm.executeTransaction(hex"<preserve_raw>");

        require(RESERVE_BALANCE.dippedIntoReserve(), "unaffected violation was cleared");
    }

    function test_execute_transaction_does_not_track_nested_only_account() public {
        vm.createSelectFork("<rpc>");

        vm.executeTransaction(hex"<nested_only_raw>");

        require(NESTED_ONLY.balance == 7 ether, "unexpected nested sender balance");
        require(!RESERVE_BALANCE.dippedIntoReserve(), "nested-only account became tracked");
    }

    function violateReserve(address account) internal {
        vm.deal(account, 12 ether);
        vm.prank(account);
        (bool success,) = payable(RECEIVER).call{value: 3 ether}("");
        require(success, "debit failed");
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected reserve violation");
    }
}
"#
    .replace("<ancestor>", &ancestor.to_string())
    .replace("<control>", &control.to_string())
    .replace("<tracked>", &tracked.to_string())
    .replace("<nested_only>", &nested_only.to_string())
    .replace("<probe>", &probe.to_string())
    .replace("<receiver>", &receiver.to_string())
    .replace("<rpc>", &handle.http_endpoint())
    .replace("<ancestor_raw>", &alloy_primitives::hex::encode(ancestor_raw))
    .replace("<control_raw>", &alloy_primitives::hex::encode(control_raw))
    .replace("<credit_raw>", &alloy_primitives::hex::encode(credit_raw))
    .replace("<preserve_raw>", &alloy_primitives::hex::encode(preserve_raw))
    .replace("<nested_only_raw>", &alloy_primitives::hex::encode(nested_only_raw));
    prj.add_test("ExecuteTransactionMonadContext.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
    });

    cmd.args(["test", "--network", "monad", "--mc", "ExecuteTransactionMonadContextTest"])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(transaction_fork_excludes_future_monad_participants, |prj, cmd| {
    use alloy_consensus::SignableTransaction as _;
    use alloy_network::{ReceiptResponse as _, TxSignerSync as _};
    use alloy_provider::Provider as _;

    const CHAIN_ID: u64 = 31_337;
    const GAS_LIMIT: u64 = 100_000;
    const MAX_FEE_PER_GAS: u128 = 3_000_000_000;

    let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let target_sender = wallets[3].address();
    let future_sender = wallets[0].address();
    let probe = alloy_primitives::Address::with_last_byte(0x21);
    let target_recipient = alloy_primitives::Address::with_last_byte(0x22);
    let future_recipient = alloy_primitives::Address::with_last_byte(0x23);
    let parent_block = provider.get_block_number().await.unwrap();

    let mut target_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 2_000_000_000,
        to: alloy_primitives::TxKind::Call(target_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[3].sign_transaction_sync(&mut target_tx).unwrap();
    let mut target_raw = Vec::new();
    target_tx.into_signed(signature).eip2718_encode(&mut target_raw);

    let mut future_marker = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(future_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut future_marker).unwrap();
    let mut future_marker_raw = Vec::new();
    future_marker.into_signed(signature).eip2718_encode(&mut future_marker_raw);

    let mut future_probe = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(probe),
        value: alloy_primitives::U256::from(3_000_000_000_000_000_000u128),
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut future_probe).unwrap();
    let mut future_probe_raw = Vec::new();
    future_probe.into_signed(signature).eip2718_encode(&mut future_probe_raw);

    let mut target_probe = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 1,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(probe),
        value: alloy_primitives::U256::from(3_000_000_000_000_000_000u128),
        ..Default::default()
    };
    let signature = wallets[3].sign_transaction_sync(&mut target_probe).unwrap();
    let mut target_probe_raw = Vec::new();
    target_probe.into_signed(signature).eip2718_encode(&mut target_probe_raw);

    let mut replayed_future_probe = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        nonce: 1,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(probe),
        value: alloy_primitives::U256::from(3_000_000_000_000_000_000u128),
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
    api.mine_one().await.unwrap();
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

    function test_non_immediate_transact_rejects_without_mutation() public {
        uint256 forkId = vm.createSelectFork("<rpc>");
        vm.rollFork(forkId, PARENT_BLOCK);
        uint64 targetNonce = vm.getNonce(TARGET_SENDER);
        uint64 futureNonce = vm.getNonce(FUTURE_SENDER);
        uint256 targetRecipientBalance = TARGET_RECIPIENT.balance;
        uint256 futureRecipientBalance = FUTURE_RECIPIENT.balance;

        bool reverted;
        try vm.transact(forkId, FUTURE_HASH) {
            reverted = false;
        } catch {
            reverted = true;
        }

        require(reverted, "non-immediate transaction was replayed");
        require(vm.getNonce(TARGET_SENDER) == targetNonce, "target nonce changed");
        require(vm.getNonce(FUTURE_SENDER) == futureNonce, "future nonce changed");
        require(TARGET_RECIPIENT.balance == targetRecipientBalance, "target balance changed");
        require(FUTURE_RECIPIENT.balance == futureRecipientBalance, "future balance changed");

        vm.transact(forkId, TARGET_HASH);
        require(vm.getNonce(TARGET_SENDER) == targetNonce + 1, "cursor changed on rejection");
        require(TARGET_RECIPIENT.balance == targetRecipientBalance + 1, "target was not committed");
        require(vm.getNonce(FUTURE_SENDER) == futureNonce, "future nonce changed after target");
        require(
            FUTURE_RECIPIENT.balance == futureRecipientBalance,
            "future balance changed after target"
        );

        vm.transact(forkId, FUTURE_HASH);
        require(vm.getNonce(FUTURE_SENDER) == futureNonce + 1, "future nonce was not advanced");
        require(
            FUTURE_RECIPIENT.balance == futureRecipientBalance + 1,
            "future was not committed"
        );
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
    .replace("<future_probe_raw>", &alloy_primitives::hex::encode(future_probe_raw))
    .replace("<target_probe_raw>", &alloy_primitives::hex::encode(target_probe_raw))
    .replace(
        "<replayed_future_probe_raw>",
        &alloy_primitives::hex::encode(replayed_future_probe_raw),
    );
    prj.add_test("TransactionForkMonadContext.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
    });

    cmd.args(["test", "--network", "monad", "--mc", "TransactionForkMonadContextTest"])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(monad_fork_aux_lifecycle_tracks_outer_context, |prj, cmd| {
    use alloy_consensus::SignableTransaction as _;
    use alloy_network::{ReceiptResponse as _, TxSignerSync as _};
    use alloy_provider::Provider as _;

    const CHAIN_ID: u64 = 31_337;
    const MAX_FEE_PER_GAS: u128 = 3_000_000_000;

    let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let sender = wallets[0].address();
    let spender = wallets[3].address();
    let unrelated = wallets[2].address();
    let marker_recipient = wallets[4].address();
    let target_recipient = wallets[5].address();
    let receiver = wallets[6].address();
    let later_marker_recipient = alloy_primitives::Address::with_last_byte(0x30);
    let later_replay_recipient = alloy_primitives::Address::with_last_byte(0x31);
    let later_target_recipient = alloy_primitives::Address::with_last_byte(0x32);
    let fresh_block = provider.get_block_number().await.unwrap();

    let mut marker_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 2_000_000_000,
        to: alloy_primitives::TxKind::Call(marker_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut marker_tx).unwrap();
    let mut marker_raw = Vec::new();
    marker_tx.into_signed(signature).eip2718_encode(&mut marker_raw);

    let mut target_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(target_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut target_tx).unwrap();
    let mut target_raw = Vec::new();
    target_tx.into_signed(signature).eip2718_encode(&mut target_raw);

    let mut credit_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(spender),
        value: alloy_primitives::U256::from(3_000_000_000_000_000_000u128),
        ..Default::default()
    };
    let signature = wallets[2].sign_transaction_sync(&mut credit_tx).unwrap();
    let mut credit_raw = Vec::new();
    credit_tx.into_signed(signature).eip2718_encode(&mut credit_raw);

    api.anvil_set_auto_mine(false).await.unwrap();
    let marker_pending = provider.send_raw_transaction(&marker_raw).await.unwrap();
    let marker_hash = *marker_pending.tx_hash();
    let target_pending = provider.send_raw_transaction(&target_raw).await.unwrap();
    let target_hash = *target_pending.tx_hash();
    api.mine_one().await.unwrap();
    let marker_receipt = marker_pending.get_receipt().await.unwrap();
    let target_receipt = target_pending.get_receipt().await.unwrap();

    let restricted_block = marker_receipt.block_number().unwrap();
    assert_eq!(restricted_block, fresh_block + 1);
    assert_eq!(marker_receipt.transaction_index(), Some(0));
    assert_eq!(target_receipt.block_number(), Some(restricted_block));
    assert_eq!(target_receipt.transaction_index(), Some(1));

    let mut later_marker_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 2_000_000_000,
        to: alloy_primitives::TxKind::Call(later_marker_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[7].sign_transaction_sync(&mut later_marker_tx).unwrap();
    let mut later_marker_raw = Vec::new();
    later_marker_tx.into_signed(signature).eip2718_encode(&mut later_marker_raw);

    let mut later_replay_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(later_replay_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[8].sign_transaction_sync(&mut later_replay_tx).unwrap();
    let mut later_replay_raw = Vec::new();
    later_replay_tx.into_signed(signature).eip2718_encode(&mut later_replay_raw);

    let mut later_target_tx = alloy_consensus::TxEip1559 {
        chain_id: CHAIN_ID,
        gas_limit: 21_000,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(later_target_recipient),
        value: alloy_primitives::U256::ONE,
        ..Default::default()
    };
    let signature = wallets[9].sign_transaction_sync(&mut later_target_tx).unwrap();
    let mut later_target_raw = Vec::new();
    later_target_tx.into_signed(signature).eip2718_encode(&mut later_target_raw);

    let later_marker_pending = provider.send_raw_transaction(&later_marker_raw).await.unwrap();
    let later_replay_pending = provider.send_raw_transaction(&later_replay_raw).await.unwrap();
    let later_replay_hash = *later_replay_pending.tx_hash();
    let later_target_pending = provider.send_raw_transaction(&later_target_raw).await.unwrap();
    let later_target_hash = *later_target_pending.tx_hash();
    api.mine_one().await.unwrap();
    let later_marker_receipt = later_marker_pending.get_receipt().await.unwrap();
    let later_replay_receipt = later_replay_pending.get_receipt().await.unwrap();
    let later_target_receipt = later_target_pending.get_receipt().await.unwrap();
    api.anvil_set_auto_mine(true).await.unwrap();

    assert_eq!(later_marker_receipt.block_number(), Some(restricted_block + 1));
    assert_eq!(later_marker_receipt.transaction_index(), Some(0));
    assert_eq!(later_replay_receipt.block_number(), Some(restricted_block + 1));
    assert_eq!(later_replay_receipt.transaction_index(), Some(1));
    assert_eq!(later_target_receipt.block_number(), Some(restricted_block + 1));
    assert_eq!(later_target_receipt.transaction_index(), Some(2));

    let upstream = handle.http_endpoint();
    let failing_replay_hash_string = later_replay_hash.to_string();
    let block_requests_armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let block_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let corrupt_replay_chain_id = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let client = reqwest::Client::new();
    let proxy_armed = std::sync::Arc::clone(&block_requests_armed);
    let proxy_requests = std::sync::Arc::clone(&block_requests);
    let proxy_corrupt_chain_id = std::sync::Arc::clone(&corrupt_replay_chain_id);
    let app = axum::Router::new().fallback(move |body: axum::body::Bytes| {
        let upstream = upstream.clone();
        let failing_replay_hash = failing_replay_hash_string.clone();
        let client = client.clone();
        let armed = std::sync::Arc::clone(&proxy_armed);
        let block_requests = std::sync::Arc::clone(&proxy_requests);
        let corrupt_chain_id = std::sync::Arc::clone(&proxy_corrupt_chain_id);
        async move {
            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
            match request.get("method").and_then(serde_json::Value::as_str) {
                Some("test_armBlockRequests") => {
                    armed.store(true, std::sync::atomic::Ordering::SeqCst)
                }
                Some("test_corruptReplayChainId") => {
                    corrupt_chain_id.store(true, std::sync::atomic::Ordering::SeqCst)
                }
                Some("test_restoreReplayChainId") => {
                    corrupt_chain_id.store(false, std::sync::atomic::Ordering::SeqCst)
                }
                _ => {
                    if armed.load(std::sync::atomic::Ordering::SeqCst) {
                        let count = request.as_array().map_or_else(
                            || {
                                usize::from(
                                    request
                                        .get("method")
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|method| method.starts_with("eth_getBlockBy")),
                                )
                            },
                            |requests| {
                                requests
                                    .iter()
                                    .filter(|request| {
                                        request
                                            .get("method")
                                            .and_then(serde_json::Value::as_str)
                                            .is_some_and(|method| {
                                                method.starts_with("eth_getBlockBy")
                                            })
                                    })
                                    .count()
                            },
                        );
                        block_requests.fetch_add(count, std::sync::atomic::Ordering::SeqCst);
                    }

                    let response = client
                        .post(upstream)
                        .header("content-type", "application/json")
                        .body(body)
                        .send()
                        .await
                        .unwrap()
                        .bytes()
                        .await
                        .unwrap();
                    if !corrupt_chain_id.load(std::sync::atomic::Ordering::SeqCst) {
                        return response;
                    }

                    let mut response: serde_json::Value =
                        serde_json::from_slice(&response).unwrap();
                    override_rpc_transaction_chain_id(&mut response, &failing_replay_hash, "0x1");
                    return axum::body::Bytes::from(serde_json::to_vec(&response).unwrap());
                }
            }

            axum::body::Bytes::from(
                serde_json::to_vec(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request.get("id").cloned().unwrap_or(serde_json::Value::Null),
                    "result": "0x",
                }))
                .unwrap(),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_endpoint = format!("http://{}", listener.local_addr().unwrap());
    let _proxy = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let source = r#"
interface Vm {
    function activeFork() external view returns (uint256);
    function broadcastRawTransaction(bytes calldata data) external;
    function createFork(string calldata url, uint256 blockNumber) external returns (uint256);
    function createSelectFork(string calldata url, uint256 blockNumber)
        external
        returns (uint256);
    function createSelectFork(string calldata url, bytes32 transaction)
        external
        returns (uint256);
    function deal(address account, uint256 newBalance) external;
    function makePersistent(address account) external;
    function prank(address msgSender) external;
    function revertToState(uint256 snapshotId) external returns (bool);
    function rpc(string calldata method, string calldata params) external returns (bytes memory);
    function rollFork(uint256 blockNumber) external;
    function rollFork(bytes32 transaction) external;
    function rollFork(uint256 forkId, uint256 blockNumber) external;
    function rollFork(uint256 forkId, bytes32 transaction) external;
    function selectFork(uint256 forkId) external;
    function snapshotState() external returns (uint256);
    function transact(bytes32 txHash) external;
    function transact(uint256 forkId, bytes32 txHash) external;
}

interface IReserveBalance {
    function dippedIntoReserve() external returns (bool);
}

contract MonadForkAuxLifecycleTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    IReserveBalance constant RESERVE_BALANCE = IReserveBalance(address(0x1001));
    string constant RPC = "<rpc>";
    address constant SENDER = <sender>;
    address constant SPENDER = <spender>;
    address constant UNRELATED = <unrelated>;
    address constant MARKER_RECIPIENT = <marker_recipient>;
    address constant LATER_MARKER_RECIPIENT = <later_marker_recipient>;
    address constant RECEIVER = <receiver>;
    uint256 constant FRESH_BLOCK = <fresh_block>;
    uint256 constant RESTRICTED_BLOCK = <restricted_block>;
    bytes32 constant MARKER_HASH = <marker_hash>;
    bytes32 constant TARGET_HASH = <target_hash>;
    bytes32 constant LATER_TARGET_HASH = <later_target_hash>;

    function test_fork_create_select_block_refreshes_fresh_to_restricted() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        vm.createSelectFork(RPC, RESTRICTED_BLOCK);
        assertSenderReserve(true);
    }

    function test_fork_create_select_block_refreshes_restricted_to_fresh() public {
        vm.createSelectFork(RPC, RESTRICTED_BLOCK);
        vm.createSelectFork(RPC, FRESH_BLOCK);
        assertSenderReserve(false);
    }

    function test_fork_create_select_hash_refreshes_fresh_to_restricted() public {
        vm.createSelectFork(RPC, MARKER_HASH);
        vm.createSelectFork(RPC, TARGET_HASH);
        assertSenderReserve(true);
    }

    function test_fork_create_select_hash_refreshes_restricted_to_fresh() public {
        vm.createSelectFork(RPC, TARGET_HASH);
        vm.createSelectFork(RPC, MARKER_HASH);
        assertSenderReserve(false);
    }

    function test_fork_select_refreshes_fresh_to_restricted() public {
        uint256 fresh = vm.createFork(RPC, FRESH_BLOCK);
        uint256 restricted = vm.createFork(RPC, RESTRICTED_BLOCK);
        vm.selectFork(fresh);
        vm.selectFork(restricted);
        assertSenderReserve(true);
    }

    function test_fork_select_refreshes_restricted_to_fresh() public {
        uint256 fresh = vm.createFork(RPC, FRESH_BLOCK);
        uint256 restricted = vm.createFork(RPC, RESTRICTED_BLOCK);
        vm.selectFork(restricted);
        vm.selectFork(fresh);
        assertSenderReserve(false);
    }

    function test_fork_select_same_id_preserves_tracker() public {
        uint256 restricted = vm.createSelectFork(RPC, RESTRICTED_BLOCK);
        debit(SENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected initial violation");

        vm.rpc("test_armBlockRequests", "[]");
        vm.selectFork(restricted);

        require(RESERVE_BALANCE.dippedIntoReserve(), "same-fork select changed tracker");
    }

    function test_fork_roll_block_refreshes_fresh_to_restricted() public {
        uint256 active = vm.createSelectFork(RPC, FRESH_BLOCK);
        vm.rollFork(active, RESTRICTED_BLOCK);
        assertSenderReserve(true);
    }

    function test_fork_roll_block_refreshes_restricted_to_fresh() public {
        vm.createSelectFork(RPC, RESTRICTED_BLOCK);
        vm.rollFork(FRESH_BLOCK);
        assertSenderReserve(false);
    }

    function test_fork_roll_hash_refreshes_fresh_to_restricted() public {
        uint256 active = vm.createSelectFork(RPC, MARKER_HASH);
        vm.rollFork(active, TARGET_HASH);
        assertSenderReserve(true);
    }

    function test_fork_roll_hash_refreshes_restricted_to_fresh() public {
        vm.createSelectFork(RPC, TARGET_HASH);
        vm.rollFork(MARKER_HASH);
        assertSenderReserve(false);
    }

    function test_fork_inactive_roll_does_not_change_active_context() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 inactive = vm.createFork(RPC, FRESH_BLOCK);

        vm.rollFork(inactive, RESTRICTED_BLOCK);
        vm.rollFork(inactive, TARGET_HASH);

        assertSenderReserve(false);
    }

    function test_fork_inactive_hash_roll_rebases_state_and_preserves_active_context() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 inactive = vm.createFork(RPC, FRESH_BLOCK);
        violateMarkerRecipient();

        vm.rollFork(inactive, TARGET_HASH);

        require(
            MARKER_RECIPIENT.balance > 10 ether - 1,
            "inactive replay state was not merged"
        );
        require(!RESERVE_BALANCE.dippedIntoReserve(), "merged credit left stale violation");
        assertSenderReserve(false);
    }

    function test_fork_active_transact_advances_outer_context() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        vm.transact(MARKER_HASH);
        assertSenderReserve(true);
    }

    function test_fork_explicit_active_transact_advances_outer_context() public {
        uint256 active = vm.createSelectFork(RPC, FRESH_BLOCK);
        vm.transact(active, MARKER_HASH);
        assertSenderReserve(true);
    }

    function test_fork_inactive_transact_rebases_state_and_preserves_active_context() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 inactive = vm.createFork(RPC, FRESH_BLOCK);
        violateMarkerRecipient();

        vm.transact(inactive, MARKER_HASH);

        require(
            MARKER_RECIPIENT.balance > 10 ether - 1,
            "inactive replay state was not merged"
        );
        require(!RESERVE_BALANCE.dippedIntoReserve(), "merged credit left stale violation");
        assertSenderReserve(false);
    }

    function violateMarkerRecipient() internal {
        vm.deal(MARKER_RECIPIENT, 10 ether);
        vm.prank(MARKER_RECIPIENT);
        (bool success,) = payable(RECEIVER).call{value: 1}("");
        require(success, "debit failed");
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected recipient violation");
    }

    function test_fork_failed_transact_preserves_outer_context() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);

        bool reverted;
        try vm.transact(TARGET_HASH) {
            reverted = false;
        } catch {
            reverted = true;
        }

        require(reverted, "non-immediate transaction was replayed");
        assertSenderReserve(false);

        // The failed target must not advance the cursor or block the immediate transaction.
        vm.transact(MARKER_HASH);
        assertSenderReserve(true);
    }

    function test_fork_failed_hash_roll_is_atomic() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 laterMarkerBalance = LATER_MARKER_RECIPIENT.balance;
        debit(SPENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected initial violation");

        bool reverted = attemptCorruptedHashRoll(0, false);

        require(reverted, "invalid replay transaction succeeded");
        require(block.number == FRESH_BLOCK, "failed roll changed block environment");
        require(
            LATER_MARKER_RECIPIENT.balance == laterMarkerBalance,
            "partial replay state leaked"
        );
        require(RESERVE_BALANCE.dippedIntoReserve(), "failed roll changed tracker");

        // The failed roll must leave the original cursor able to execute its immediate target.
        vm.transact(MARKER_HASH);
        assertSenderReserve(true);
    }

    function test_fork_failed_hash_roll_retries_without_partial_replay() public {
        uint256 active = vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 laterMarkerBalance = LATER_MARKER_RECIPIENT.balance;

        require(attemptCorruptedHashRoll(active, true), "invalid replay transaction succeeded");
        require(
            LATER_MARKER_RECIPIENT.balance == laterMarkerBalance,
            "partial replay state leaked"
        );

        vm.rollFork(active, LATER_TARGET_HASH);

        require(
            LATER_MARKER_RECIPIENT.balance == laterMarkerBalance + 1,
            "successful retry did not replay predecessor exactly once"
        );
    }

    function test_fork_failed_inactive_hash_roll_is_atomic() public {
        uint256 active = vm.createSelectFork(RPC, FRESH_BLOCK);
        uint256 inactive = vm.createFork(RPC, FRESH_BLOCK);
        uint256 laterMarkerBalance = LATER_MARKER_RECIPIENT.balance;
        debit(SPENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected initial violation");

        require(attemptCorruptedHashRoll(inactive, true), "invalid replay transaction succeeded");
        require(vm.activeFork() == active, "failed roll changed active fork");
        require(block.number == FRESH_BLOCK, "failed roll changed block environment");
        require(
            LATER_MARKER_RECIPIENT.balance == laterMarkerBalance,
            "inactive partial replay state leaked"
        );
        require(RESERVE_BALANCE.dippedIntoReserve(), "failed roll changed tracker");

        vm.selectFork(inactive);
        vm.transact(MARKER_HASH);
        assertSenderReserve(true);
    }

    function attemptCorruptedHashRoll(uint256 forkId, bool explicitFork)
        internal
        returns (bool reverted)
    {
        vm.rpc("test_corruptReplayChainId", "[]");
        if (explicitFork) {
            try vm.rollFork(forkId, LATER_TARGET_HASH) {
                reverted = false;
            } catch {
                reverted = true;
            }
        } else {
            try vm.rollFork(LATER_TARGET_HASH) {
                reverted = false;
            } catch {
                reverted = true;
            }
        }
        vm.rpc("test_restoreReplayChainId", "[]");
    }

    function test_broadcast_raw_transaction_rebases_without_advancing_cursor() public {
        vm.createSelectFork(RPC, FRESH_BLOCK);
        debit(SPENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected initial violation");

        vm.broadcastRawTransaction(hex"<credit_raw>");

        require(SPENDER.balance == 12 ether, "unexpected credited balance");
        require(!RESERVE_BALANCE.dippedIntoReserve(), "credited violation was not cleared");

        vm.transact(MARKER_HASH);
        assertSenderReserve(true);
    }

    function test_fork_persistent_violation_survives_select() public {
        uint256 first = vm.createFork(RPC, FRESH_BLOCK);
        uint256 second = vm.createFork(RPC, FRESH_BLOCK);
        vm.selectFork(first);
        vm.makePersistent(SPENDER);
        debit(SPENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected persistent violation");

        vm.selectFork(second);

        require(RESERVE_BALANCE.dippedIntoReserve(), "persistent violation was dropped");
    }

    function test_fork_nonpersistent_violation_drops_on_select() public {
        uint256 first = vm.createFork(RPC, FRESH_BLOCK);
        uint256 second = vm.createFork(RPC, FRESH_BLOCK);
        vm.selectFork(first);
        debit(SPENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected old-fork violation");

        vm.selectFork(second);

        require(!RESERVE_BALANCE.dippedIntoReserve(), "old-fork violation leaked");
    }

    function test_fork_unrelated_loaded_account_remains_untracked() public {
        uint256 first = vm.createFork(RPC, FRESH_BLOCK);
        uint256 second = vm.createFork(RPC, FRESH_BLOCK);
        vm.selectFork(first);
        vm.makePersistent(UNRELATED);
        vm.deal(UNRELATED, 9 ether);

        vm.selectFork(second);

        require(!RESERVE_BALANCE.dippedIntoReserve(), "loaded account became tracked");
    }

    function test_fork_snapshot_restores_chain_and_tracker() public {
        uint256 fresh = vm.createFork(RPC, FRESH_BLOCK);
        uint256 restricted = vm.createFork(RPC, RESTRICTED_BLOCK);
        vm.selectFork(restricted);
        debit(SENDER);
        require(RESERVE_BALANCE.dippedIntoReserve(), "expected snapshotted violation");
        uint256 snapshot = vm.snapshotState();

        vm.selectFork(fresh);
        require(!RESERVE_BALANCE.dippedIntoReserve(), "fresh context retained violation");

        require(vm.revertToState(snapshot), "snapshot revert failed");
        require(RESERVE_BALANCE.dippedIntoReserve(), "snapshot did not restore tracker");
    }

    function assertSenderReserve(bool expected) internal {
        debit(SENDER);
        require(
            RESERVE_BALANCE.dippedIntoReserve() == expected,
            "unexpected sender reserve state"
        );
    }

    function debit(address account) internal {
        vm.deal(account, 12 ether);
        vm.prank(account);
        (bool success,) = payable(RECEIVER).call{value: 3 ether}("");
        require(success, "debit failed");
    }
}
"#
    .replace("<rpc>", &rpc_endpoint)
    .replace("<sender>", &sender.to_string())
    .replace("<spender>", &spender.to_string())
    .replace("<unrelated>", &unrelated.to_string())
    .replace("<marker_recipient>", &marker_recipient.to_string())
    .replace("<later_marker_recipient>", &later_marker_recipient.to_string())
    .replace("<receiver>", &receiver.to_string())
    .replace("<fresh_block>", &fresh_block.to_string())
    .replace("<restricted_block>", &restricted_block.to_string())
    .replace("<marker_hash>", &marker_hash.to_string())
    .replace("<target_hash>", &target_hash.to_string())
    .replace("<later_target_hash>", &later_target_hash.to_string())
    .replace("<credit_raw>", &alloy_primitives::hex::encode(credit_raw));
    prj.add_test("MonadForkAuxLifecycle.t.sol", &source);
    prj.update_config(|config| {
        config.hardfork = Some("monad:MonadNine".parse().unwrap());
        config.sender = sender;
    });

    cmd.args(["test", "--network", "monad", "--mc", "MonadForkAuxLifecycleTest", "--threads", "1"])
        .assert_success();

    block_requests_armed.store(false, std::sync::atomic::Ordering::SeqCst);
    block_requests.store(0, std::sync::atomic::Ordering::SeqCst);
    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--mc",
            "MonadForkAuxLifecycleTest",
            "--mt",
            "test_fork_select_same_id_preserves_tracker",
        ])
        .assert_success();
    assert_eq!(
        block_requests.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "same-active select fetched a block"
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--network",
            "monad",
            "--mc",
            "MonadForkAuxLifecycleTest",
            "--mt",
            "test_fork_active_transact_advances_outer_context",
            "--isolate",
        ])
        .assert_success();
});

#[cfg(feature = "monad")]
forgetest_async!(transact_replays_monad_protocol_system_target_forks, |prj, cmd| {
    use alloy_network::{ReceiptResponse as _, TransactionBuilder as _};
    use alloy_provider::Provider as _;

    const SYSTEM_ADDRESS: alloy_primitives::Address =
        alloy_primitives::address!("0x6f49a8F621353f12378d0046E7d7e4b9B249DC9e");
    const STAKING_ADDRESS: alloy_primitives::Address =
        alloy_primitives::address!("0x0000000000000000000000000000000000001000");
    const BLOCK_AUTHOR: alloy_primitives::Address =
        alloy_primitives::address!("0x1111111111111111111111111111111111111111");
    const VALIDATOR_AUTH: alloy_primitives::Address =
        alloy_primitives::address!("0x2222222222222222222222222222222222222222");
    const UNKNOWN_BLOCK_AUTHOR: alloy_primitives::Address =
        alloy_primitives::address!("0x3333333333333333333333333333333333333333");
    const VALIDATOR_ID: u64 = 7;

    let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let provider = handle.http_provider();
    let mon = alloy_primitives::U256::from(1_000_000_000_000_000_000u128);
    let reward = alloy_primitives::U256::from(25) * mon;
    let initial_system_balance = alloy_primitives::U256::from(100) * mon;
    let initial_staking_balance = alloy_primitives::U256::from(3) * mon;
    api.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();
    api.anvil_set_nonce(SYSTEM_ADDRESS, alloy_primitives::U256::from(11)).await.unwrap();
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
        storage_value(alloy_primitives::U256::from(100) * mon),
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        monad_staking_validator_key(0x04, VALIDATOR_ID, 1),
        alloy_primitives::B256::ZERO,
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

    api.mine_one().await.unwrap();
    let parent_block = provider.get_block_number().await.unwrap();

    let request =
        <alloy_network::Ethereum as alloy_network::Network>::TransactionRequest::default()
            .with_from(SYSTEM_ADDRESS)
            .with_to(STAKING_ADDRESS)
            .with_value(reward)
            .with_input(monad_staking_reward_input(BLOCK_AUTHOR))
            .with_gas_limit(1_000_000)
            .with_gas_price(2_000_000_000);
    let receipt =
        provider.send_transaction(request.into()).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
    assert_eq!(receipt.block_number(), Some(parent_block + 1));

    let target_hash = receipt.transaction_hash;
    let endpoint = foundry_test_utils::rpc::spawn_canonical_monad_system_rpc(
        handle.http_endpoint(),
        target_hash,
    )
    .await;
    let transaction =
        rpc_request(&endpoint, "eth_getTransactionByHash", serde_json::json!([target_hash])).await;
    assert_eq!(transaction["result"]["gas"], "0x0");
    assert_eq!(transaction["result"]["gasPrice"], "0x0");
    assert_ne!(transaction["result"]["r"], "0x0");
    assert_ne!(transaction["result"]["s"], "0x0");
    assert_eq!(transaction["result"]["type"], "0x0");
    assert_ne!(transaction["result"]["v"], "0x0");
    assert_eq!(transaction["result"]["value"], format!("{reward:#x}"));

    let canonical_receipt =
        rpc_request(&endpoint, "eth_getTransactionReceipt", serde_json::json!([target_hash])).await;
    assert_eq!(canonical_receipt["result"]["status"], "0x1");
    assert_eq!(canonical_receipt["result"]["gasUsed"], "0x0");
    assert_eq!(canonical_receipt["result"]["effectiveGasPrice"], "0x0");

    let target_block = rpc_request(
        &endpoint,
        "eth_getBlockByNumber",
        serde_json::json!([format!("{:#x}", parent_block + 1), true]),
    )
    .await;
    assert_ne!(target_block["result"]["baseFeePerGas"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["hash"], target_hash.to_string());
    assert_eq!(target_block["result"]["transactions"][0]["gas"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["gasPrice"], "0x0");
    assert_eq!(target_block["result"]["transactions"][0]["r"], transaction["result"]["r"]);
    assert_eq!(target_block["result"]["transactions"][0]["s"], transaction["result"]["s"]);
    assert_eq!(target_block["result"]["transactions"][0]["v"], transaction["result"]["v"]);

    let (failed_api, failed_handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let failed_provider = failed_handle.http_provider();
    failed_api.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();
    failed_api.anvil_set_nonce(SYSTEM_ADDRESS, alloy_primitives::U256::from(11)).await.unwrap();
    failed_api.anvil_set_balance(SYSTEM_ADDRESS, initial_system_balance).await.unwrap();
    failed_api.anvil_set_balance(STAKING_ADDRESS, initial_staking_balance).await.unwrap();
    failed_api.mine_one().await.unwrap();
    let failed_parent_block = failed_provider.get_block_number().await.unwrap();
    let failed_request =
        <alloy_network::Ethereum as alloy_network::Network>::TransactionRequest::default()
            .with_from(SYSTEM_ADDRESS)
            .with_to(STAKING_ADDRESS)
            .with_value(reward)
            .with_input(monad_staking_reward_input(UNKNOWN_BLOCK_AUTHOR))
            .with_gas_limit(1_000_000)
            .with_gas_price(2_000_000_000);
    let failed_receipt = failed_provider
        .send_transaction(failed_request.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(failed_receipt.status());
    assert_eq!(failed_receipt.block_number(), Some(failed_parent_block + 1));
    let failed_target_hash = failed_receipt.transaction_hash;
    let failed_endpoint = foundry_test_utils::rpc::spawn_canonical_monad_system_rpc(
        failed_handle.http_endpoint(),
        failed_target_hash,
    )
    .await;

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
    bytes32 constant FAILED_TARGET_HASH = <failed_tx_hash>;
    uint256 constant PARENT_BLOCK = <parent_block>;
    uint256 constant FAILED_PARENT_BLOCK = <failed_parent_block>;
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

    function test_failed_reward_target_from_transaction_hash_fork_rolls_back() public {
        vm.createSelectFork("<failed_rpc>", FAILED_TARGET_HASH);
        _assertFailedRewardRollback();
    }

    function test_failed_reward_target_from_parent_block_fork_rolls_back() public {
        vm.createSelectFork("<failed_rpc>", FAILED_PARENT_BLOCK);
        _assertFailedRewardRollback();
    }

    function _assertFailedRewardRollback() internal {
        uint256 systemBalanceBefore = SYSTEM.balance;
        uint256 stakingBalanceBefore = address(STAKING).balance;
        uint64 nonceBefore = vm.getNonce(SYSTEM);

        bool reverted;
        try vm.transact(FAILED_TARGET_HASH) {
            reverted = false;
        } catch {
            reverted = true;
        }

        require(reverted, "invalid reward target was replayed");
        require(SYSTEM.balance == systemBalanceBefore, "protocol caller balance changed");
        require(address(STAKING).balance == stakingBalanceBefore, "reward mint was committed");
        require(vm.getNonce(SYSTEM) == nonceBefore, "protocol caller nonce was committed");
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
    .replace("<failed_rpc>", &failed_endpoint)
    .replace("<origin_rpc>", &handle.http_endpoint())
    .replace("<tx_hash>", &target_hash.to_string())
    .replace("<failed_tx_hash>", &failed_target_hash.to_string())
    .replace("<parent_block>", &parent_block.to_string())
    .replace("<failed_parent_block>", &failed_parent_block.to_string());
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
