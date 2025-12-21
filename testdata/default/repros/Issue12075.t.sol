// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/12075
contract Issue12075Test is Test {
    address payable internal ALICE = payable(address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266));
    address payable internal BOB = payable(address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8));

    Target internal target;

    function setUp() public virtual {
        target = new Target();

        vm.deal({account: ALICE, newBalance: 100 ether});
        vm.startPrank(ALICE);
    }

    function test_NativeTransfer() public {
        BOB.transfer(1 ether);
        assertEq(BOB.balance, 1 ether);
    }

    function test_PayableFunction() public {
        target.hit{value: 1 wei}();
    }
}

contract Target {
    function hit() public payable {}
}
