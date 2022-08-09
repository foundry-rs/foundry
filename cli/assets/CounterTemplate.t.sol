// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;
    function setUp() public {
       counter = new Counter();
       counter.setNumber(0);
    }

    function testIncrement() public {
        counter.increment();
        assertEq(counter.number(),1);
    }

    function testSetNumber() public {
        counter.setNumber(10);
        assertEq(counter.number(),10);
    }
}
