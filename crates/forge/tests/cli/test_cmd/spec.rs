use foundry_test_utils::rpc;

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
