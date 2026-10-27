// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Child {}

contract Bumper {
    function create() external {
        new Child();
    }
}

/// `deal` and nonce cheatcodes survive reverts of the enclosing calls, and compose with the
/// transfers and nonce bumps those reverts undo, as on master.
contract PersistentWritesTest is Test {
    address payable constant A = payable(address(0xA11CE));
    Bumper bumper;

    function setUp() public {
        vm.deal(address(this), 1 ether);
        bumper = new Bumper();
    }

    function dealTwice() external {
        vm.deal(A, 1);
        vm.deal(A, 2);
        revert();
    }

    function transferThenDeal() external {
        A.transfer(10);
        vm.deal(A, 100);
        revert();
    }

    function dealThenTransfer() external {
        vm.deal(A, 100);
        A.transfer(10);
        revert();
    }

    function dealInChild() external {
        this.dealOk();
        revert();
    }

    function dealOk() external {
        vm.deal(A, 100);
    }

    function dealInRevertedChild() external {
        try this.dealTwice() {} catch {}
        A.transfer(10);
        revert();
    }

    function createThenSetNonce() external {
        bumper.create();
        vm.setNonce(address(bumper), 10);
        revert();
    }

    function setNonceTwice() external {
        vm.setNonce(A, 5);
        vm.setNonce(A, 7);
        revert();
    }

    function callReverting(bytes memory data) internal {
        (bool ok,) = address(this).call(data);
        require(!ok, "expected revert");
    }

    function balanceAfter(bytes memory data) internal returns (uint256) {
        vm.deal(A, 50);
        callReverting(data);
        return A.balance;
    }

    function testWritesSurviveReverts() public {
        assertEq(balanceAfter(abi.encodeCall(this.dealTwice, ())), 2, "dealTwice");
        assertEq(balanceAfter(abi.encodeCall(this.transferThenDeal, ())), 90, "transferThenDeal");
        assertEq(balanceAfter(abi.encodeCall(this.dealThenTransfer, ())), 100, "dealThenTransfer");
        assertEq(balanceAfter(abi.encodeCall(this.dealInChild, ())), 100, "dealInChild");
        assertEq(balanceAfter(abi.encodeCall(this.dealInRevertedChild, ())), 2, "dealInRevertedChild");

        callReverting(abi.encodeCall(this.createThenSetNonce, ()));
        assertEq(vm.getNonce(address(bumper)), 9, "createThenSetNonce");
        callReverting(abi.encodeCall(this.setNonceTwice, ()));
        assertEq(vm.getNonce(A), 7, "setNonceTwice");
    }
}
