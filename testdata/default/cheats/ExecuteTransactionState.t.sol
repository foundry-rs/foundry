// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// forge-config: default.isolate = true
contract ExecuteTransactionStateTest is Test {
    address constant SENDER = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
    address constant TARGET = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
    bytes constant RAW_TX =
        hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";
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

    function executeAfterOrdinaryWrites() external {
        (bool success,) = TARGET.call{value: 10}(abi.encode(uint256(5)));
        require(success, "ordinary call failed");
        assertEq(vm.load(TARGET, bytes32(0)), bytes32(uint256(5)));
        vm.executeTransaction(RAW_TX);
        assertEq(vm.load(TARGET, bytes32(0)), bytes32(0));
        revert("parent reverted");
    }

    function test_execute_preserves_overlapping_ordinary_undo() public {
        vm.chainId(1);
        vm.fee(1);
        vm.deal(SENDER, 1 ether);
        vm.deal(address(this), 10);
        vm.deal(TARGET, 50);
        // Store the first calldata word; the signed transaction has empty calldata.
        vm.etch(TARGET, hex"60003560005500");
        vm.store(TARGET, bytes32(0), bytes32(uint256(3)));
        try this.executeAfterOrdinaryWrites() {
            revert("expected parent revert");
        } catch Error(string memory reason) {
            assertEq(reason, "parent reverted");
        }
        assertEq(TARGET.balance, 67);
        assertEq(address(this).balance, 10);
        assertEq(vm.load(TARGET, bytes32(0)), bytes32(uint256(3)));
        assertEq(vm.getNonce(SENDER), 1);
    }

    function executeAfterDelegation() external {
        vm.signAndAttachDelegation(address(0), 12345);
        vm.executeTransaction(RAW_TX);
        assertEq(vm.addr(12345).code, hex"00");
        revert("parent reverted");
    }

    function test_execute_preserves_same_value_code_undo() public {
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        vm.allowCheatcodes(TARGET);
        vm.etch(TARGET, type(NestedCodeWriter).runtimeCode);
        try this.executeAfterDelegation() {
            revert("expected parent revert");
        } catch Error(string memory reason) {
            assertEq(reason, "parent reverted");
        }
        assertEq(vm.addr(12345).code, hex"");
    }
}

/// forge-config: default.isolate = false
contract ExecuteTransactionStateNonIsolatedTest is ExecuteTransactionStateTest {}

contract NestedCodeWriter is Test {
    fallback() external payable {
        vm.etch(vm.addr(12345), hex"00");
    }
}
