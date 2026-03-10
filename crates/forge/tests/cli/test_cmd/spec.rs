use foundry_test_utils::rpc;
use monad_revm::MonadSpecId;

// Test evm version behavior during tests / scripts.
// On Monad, Ethereum evm_version strings are compatibility no-ops, while Monad hardfork names can
// explicitly select a Monad execution spec.
// Original upstream refs:
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

        // On Monad (MONAD_EIGHT / Prague-compatible), CREATE2 is always available
        // even after setEvmVersion to older versions.
        compute();

        evm.setEvmVersion("shanghai");
        evm.getEvmVersion();
        compute();

        evm.setEvmVersion("paris");
        compute();
    }

    function compute() internal view {
        ICreate2Deployer(0x35Da41c476fA5c6De066f20556069096A1F39364).computeAddress(bytes32(0), bytes32(0));
    }
}
   "#.replace("<rpc>", &endpoint),
    );

    cmd.args(["test", "--mc", "TestEvmVersion", "-vvvv"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/TestEvmVersion.t.sol:TestEvmVersion
[PASS] test_evm_version() ([GAS])
...
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test evm version set in `setUp` — on Monad, setEvmVersion("istanbul") has no effect,
    // so CREATE2 remains available and the test succeeds.
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
        // On Monad, CREATE2 is always available (MONAD_EIGHT is Prague-compatible).
        ICreate2Deployer(0x35Da41c476fA5c6De066f20556069096A1F39364).computeAddress(bytes32(0), bytes32(0));
    }
}
   "#.replace("<rpc>", &endpoint),
    );
    cmd.forge_fuse()
        .args(["test", "--mc", "TestSetupEvmVersion", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] test_evm_version_in_setup() ([GAS])
...
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test evm version set in constructor — on Monad, setEvmVersion("istanbul") has no effect,
    // so CREATE2 remains available and the test succeeds.
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
        // On Monad, CREATE2 is always available (MONAD_EIGHT is Prague-compatible).
        ICreate2Deployer(0x35Da41c476fA5c6De066f20556069096A1F39364).computeAddress(bytes32(0), bytes32(0));
    }
}
   "#.replace("<rpc>", &endpoint),
    );
    cmd.forge_fuse()
        .args(["test", "--mc", "TestConstructorEvmVersion", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] test_evm_version_in_constructor() ([GAS])
...
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    prj.update_config(|config| {
        config.monad_hardfork = Some(MonadSpecId::MonadNine);
    });
    prj.add_test(
        "TestMonadHardforkEvmVersion.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface EvmVm {
    function getEvmVersion() external pure returns (string memory evm);
    function setEvmVersion(string calldata evm) external;
}

contract TestMonadHardforkEvmVersion is Test {
    EvmVm constant evm = EvmVm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function test_monad_hardfork_override() public {
        assertEq(evm.getEvmVersion(), "monadnine");

        evm.setEvmVersion("istanbul");
        assertEq(evm.getEvmVersion(), "monadnine");

        evm.setEvmVersion("MonadEight");
        assertEq(evm.getEvmVersion(), "monadeight");

        evm.setEvmVersion("paris");
        assertEq(evm.getEvmVersion(), "monadeight");

        evm.setEvmVersion("MonadNine");
        assertEq(evm.getEvmVersion(), "monadnine");
    }
}
   "#,
    );
    cmd.forge_fuse()
        .args(["test", "--mc", "TestMonadHardforkEvmVersion", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] test_monad_hardfork_override() ([GAS])
...
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});
