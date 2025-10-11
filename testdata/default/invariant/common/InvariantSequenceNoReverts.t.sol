// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "utils/Test.sol";

contract SequenceNoReverts {
    uint256 public count;

    function work(uint256 x) public {
        require(x % 2 != 0);
        count++;
    }
}

contract SequenceNoRevertsTest is Test {
    SequenceNoReverts target;

    function setUp() public {
        target = new SequenceNoReverts();
    }

    function invariant_no_reverts() public view {
        require(target.count() < 10, "condition met");
    }
}
