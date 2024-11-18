// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";

contract Counter {
    uint256 public number;

    function increment() public {
        number++;
    }

    function decrement() public {
        number--;
    }
}

contract InvariantShrinkWithAssert is DSTest {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_with_assert() public {
        assertTrue(counter.number() < 2, "wrong counter");
    }

    function invariant_with_require() public {
        require(counter.number() < 2, "wrong counter");
    }
}
