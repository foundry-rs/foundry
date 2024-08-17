// SPDX-License-Identifier: MIT
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}

// https://github.com/foundry-rs/foundry/issues/6115
contract Issue6115Test is DSTest {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    // We should be able to fuzz bytes4
    function testFuzz_SetNumber(uint256 x, bytes4 test) public {
        counter.setNumber(x);
        assertEq(counter.number(), x);
    }

    // We should be able to fuzz bytes8
    function testFuzz_SetNumber2(uint256 x, bytes8 test) public {
        counter.setNumber(x);
        assertEq(counter.number(), x);
    }

    // We should be able to fuzz bytes12
    function testFuzz_SetNumber3(uint256 x, bytes12 test) public {
        counter.setNumber(x);
        assertEq(counter.number(), x);
    }
}
