// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SelfDestructTo {
    constructor(address target) payable {
        selfdestruct(payable(target));
    }
}

contract TransferOverflowTest is Test {
    address constant DST = address(0x1234);

    // A call whose value overflows the recipient's balance fails without moving any value.
    function testCallValueOverflowFails() public {
        vm.deal(address(this), 2);
        vm.deal(DST, type(uint256).max);
        (bool ok, bytes memory data) = DST.call{value: 1}("");
        assertTrue(!ok);
        assertEq(data.length, 0);
        assertEq(address(this).balance, 2);
        assertEq(DST.balance, type(uint256).max);
    }

    // SELFDESTRUCT always moves the whole balance and wraps the beneficiary's balance, as in revm.
    function testSelfDestructOverflowWraps() public {
        vm.deal(address(this), 3);
        vm.deal(DST, type(uint256).max);
        new SelfDestructTo{value: 2}(DST);
        assertEq(address(this).balance, 1);
        assertEq(DST.balance, 1);
    }
}
