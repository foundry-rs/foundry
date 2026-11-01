// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

address constant SENDER = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
address constant TARGET = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
// Transfers 17 wei from `SENDER` to `TARGET` at nonce 0 on chain 1.
bytes constant RAW_TX =
    hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";

contract ExecuteTransactionSelfDestructTest is Test {
    function beforeTestSetup(bytes4) public view returns (bytes[] memory calls) {
        calls = new bytes[](1);
        calls[0] = abi.encodeCall(this.destroyInRevertedCall, ());
    }

    function destroyInRevertedCall() external {
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        // Self-destructs to the zero address.
        vm.etch(TARGET, hex"6000ff");
        vm.store(TARGET, bytes32(0), bytes32(uint256(9)));
        try this.executeThenRevert() {
            revert("expected parent revert");
        } catch Error(string memory reason) {
            assertEq(reason, "parent reverted");
        }
    }

    function executeThenRevert() external {
        vm.executeTransaction(RAW_TX);
        revert("parent reverted");
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = true
    function test_execute_selfdestruct_survives_parent_revert() public {
        checkDestroyed();
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = false
    function test_execute_selfdestruct_survives_parent_revert_non_isolated() public {
        checkDestroyed();
    }

    function checkDestroyed() internal {
        // The destruction applied when the setup transaction committed.
        assertEq(TARGET.code.length, 0);
        assertEq(vm.load(TARGET, bytes32(0)), bytes32(0));
    }
}

/// Before the test transaction ends, an account the nested transaction self-destructed keeps the
/// state the transaction left it in, also when an enclosing call reverts.
contract ExecuteTransactionSelfDestructInTransactionTest is Test {
    function executeThenRevert() external {
        // The nested transaction only reads slot 2, so the revert still undoes this store.
        vm.store(TARGET, bytes32(uint256(2)), bytes32(uint256(42)));
        vm.executeTransaction(RAW_TX);
        revert();
    }

    /// forge-config: default.evm_version = "paris"
    /// forge-config: default.isolate = false
    function test_execute_selfdestruct_keeps_state_in_transaction() public {
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        vm.deal(TARGET, 5);
        // `SLOAD(2)`, `SSTORE(0, 1)`, then `SELFDESTRUCT(0)`.
        vm.etch(TARGET, hex"6002545060016000556000ff");
        vm.store(TARGET, 0, bytes32(uint256(9)));
        vm.store(TARGET, bytes32(uint256(2)), bytes32(uint256(4)));
        uint256 beneficiary = address(0).balance;
        try this.executeThenRevert() {} catch {}
        assertEq(TARGET.code.length, 12, "code");
        assertEq(uint256(vm.load(TARGET, 0)), 1, "slot0");
        assertEq(uint256(vm.load(TARGET, bytes32(uint256(2)))), 4, "slot2");
        assertEq(TARGET.balance, 0, "balance");
        assertEq(address(0).balance - beneficiary, 22, "beneficiary");
        assertEq(vm.getNonce(SENDER), 1, "nonce");
    }
}
