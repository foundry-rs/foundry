// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Child {}

contract Bumper {
    function create() external {
        new Child();
    }
}

contract ScratchPersistentWritesTest is Test {
    address payable constant A = payable(address(0xA11CE));
    Bumper bumper;

    function setUp() public {
        vm.deal(A, 50);
        vm.deal(address(this), 1 ether);
        bumper = new Bumper();
    }

    function expectRevert(bytes memory data) internal {
        (bool ok,) = address(this).call(data);
        require(!ok, "expected revert");
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

    function test_dealTwice() public {
        expectRevert(abi.encodeCall(this.dealTwice, ()));
        emit log_named_uint("dealTwice balance", A.balance);
    }

    function test_transferThenDeal() public {
        expectRevert(abi.encodeCall(this.transferThenDeal, ()));
        emit log_named_uint("transferThenDeal balance", A.balance);
    }

    function test_dealThenTransfer() public {
        expectRevert(abi.encodeCall(this.dealThenTransfer, ()));
        emit log_named_uint("dealThenTransfer balance", A.balance);
    }

    function test_createThenSetNonce() public {
        expectRevert(abi.encodeCall(this.createThenSetNonce, ()));
        emit log_named_uint("createThenSetNonce nonce", vm.getNonce(address(bumper)));
    }

    function test_setNonceTwice() public {
        expectRevert(abi.encodeCall(this.setNonceTwice, ()));
        emit log_named_uint("setNonceTwice nonce", vm.getNonce(A));
    }

    function test_dealInChild() public {
        expectRevert(abi.encodeCall(this.dealInChild, ()));
        emit log_named_uint("dealInChild balance", A.balance);
    }

    function test_dealInRevertedChild() public {
        expectRevert(abi.encodeCall(this.dealInRevertedChild, ()));
        emit log_named_uint("dealInRevertedChild balance", A.balance);
    }
}
