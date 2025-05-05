// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ICounter} from "../src/interface/ICounter.sol";
import {VyperDeployer} from "../src/utils/VyperDeployer.sol";

contract CounterTest is Test {
    VyperDeployer public vyperDeployer;
    ICounter public counterContract;
    uint256 public constant INITIAL_COUNTER = 42;

    function setUp() public {
        vyperDeployer = new VyperDeployer();
        counterContract = ICounter(vyperDeployer.deployContract("Counter", abi.encode(INITIAL_COUNTER)));
    }

    function test_getCounter() public view {
        uint256 counter = counterContract.counter();
        assertEq(counter, INITIAL_COUNTER);
    }

    function test_setCounter() public {
        uint256 newCounter = 100;
        counterContract.set_counter(newCounter);
        uint256 counter = counterContract.counter();
        assertEq(counter, newCounter);
    }

    function test_increment() public {
        uint256 counterBefore = counterContract.counter();
        counterContract.increment();
        uint256 counterAfter = counterContract.counter();
        assertEq(counterAfter, counterBefore + 1);
    }
}
