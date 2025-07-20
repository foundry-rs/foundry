// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";
// import {IPowerCalculator} from "../out/PowerCalculator.wasm/interface.sol";

contract CounterTest is Test {
    Counter public counter;
    address public powerCalculator;

    function setUp() public {
        // Deploy the WASM PowerCalculator contract
        powerCalculator = vm.deployCode("PowerCalculator.wasm");
        
        // Deploy Counter with the PowerCalculator address
        counter = new Counter(powerCalculator);
    }

    function testInitialNumber() public view{
        assertEq(counter.number(), 1);
    }

    function testIncrement() public {
        counter.increment();
        assertEq(counter.number(), 2);
    }

    function testSetNumber() public {
        counter.setNumber(42);
        assertEq(counter.number(), 42);
    }

    function testIncrementByPowerOfTwo() public {
        // number = 1, increment by 2^3 = 8
        counter.incrementByPowerOfTwo(3);
        assertEq(counter.number(), 9); // 1 + 8 = 9

        // increment by 2^4 = 16
        counter.incrementByPowerOfTwo(4);
        assertEq(counter.number(), 25); // 9 + 16 = 25
    }

    function testSetNumberToPower() public {
        // Set to 3^4 = 81
        counter.setNumberToPower(3, 4);
        assertEq(counter.number(), 81);

        // Set to 5^3 = 125
        counter.setNumberToPower(5, 3);
        assertEq(counter.number(), 125);
    }

    function testCurrentNumberToPower() public {
        counter.setNumber(2);
        assertEq(counter.currentNumberToPower(3), 8); // 2^3 = 8

        counter.setNumber(10);
        assertEq(counter.currentNumberToPower(2), 100); // 10^2 = 100
    }
}