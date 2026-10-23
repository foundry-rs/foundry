// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// forge-config: default.isolate = true
contract ExecuteTransactionStateTest is Test {
    address constant SENDER = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
    address constant TARGET = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
    bytes constant RAW_TX =
        hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";
    address constant CREATOR = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf;
    // Deploys runtime code `602a60005260206000f3` from `CREATOR` at nonce 0 on chain 1.
    bytes constant CREATE_TX =
        hex"f862806483030d40808096600a600c600039600a6000f3602a60005260206000f326a0aa15e60239616b411e2ad4409d0017b52ec731ca00100bf59a2d73e3f692123ca07d7aec8bdb1d2202e5c618a6542e889b3e4ab352606232fb40d7b67521791366";
    uint256 public parentValue;

    function test_execute_success_preserves_parent_state() public {
        checkState(hex"600960005500", "", false);
    }

    function test_execute_success_survives_parent_revert() public {
        checkState(hex"600960005500", "", true);
    }

    function test_execute_revert_preserves_nonce_without_fees() public {
        checkState(hex"600960005560006000fd", "transaction reverted: 0x", false);
    }

    function test_execute_halt_preserves_nonce_without_fees() public {
        checkState(hex"6009600055fe", "transaction halted: InvalidFEOpcode", false);
    }

    function executeInParent(string calldata expectedError, bool revertParent) external {
        parentValue = 7;
        try vm.executeTransaction(RAW_TX) {
            assertEq(bytes(expectedError).length, 0);
        } catch (bytes memory reason) {
            assertGt(bytes(expectedError).length, 0);
            assertEq(
                reason,
                abi.encodeWithSignature(
                    "CheatcodeError(string)", string.concat("vm.executeTransaction: ", expectedError)
                )
            );
        }
        if (revertParent) revert("parent reverted");
    }

    function executeCreateThenRevert() external {
        vm.executeTransaction(CREATE_TX);
        revert("parent reverted");
    }

    function test_execute_create_survives_parent_revert() public {
        vm.chainId(1);
        vm.deal(CREATOR, 1 ether);
        try this.executeCreateThenRevert() {
            revert("expected parent revert");
        } catch Error(string memory reason) {
            assertEq(reason, "parent reverted");
        }
        assertEq(vm.getNonce(CREATOR), 1);
        assertEq(vm.computeCreateAddress(CREATOR, 0).code, hex"602a60005260206000f3");
    }

    function checkState(bytes memory code, string memory expectedError, bool revertParent) internal {
        vm.chainId(1);
        vm.fee(1);
        vm.deal(SENDER, 1 ether);
        vm.etch(TARGET, code);
        try this.executeInParent(expectedError, revertParent) {
            assertTrue(!revertParent);
        } catch Error(string memory reason) {
            assertTrue(revertParent);
            assertEq(reason, "parent reverted");
        }
        bool success = bytes(expectedError).length == 0;
        assertEq(vm.getNonce(SENDER), 1);
        assertEq(SENDER.balance, 1 ether - (success ? 17 : 0));
        assertEq(TARGET.balance, success ? 17 : 0);
        assertEq(vm.load(TARGET, bytes32(0)), bytes32(success ? uint256(9) : 0));
        assertEq(parentValue, revertParent ? 0 : 7);
    }
}

/// forge-config: default.isolate = false
contract ExecuteTransactionStateNonIsolatedTest is ExecuteTransactionStateTest {}
