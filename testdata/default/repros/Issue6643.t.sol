// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Counter {
    event TestEvent(uint256 n);
    event AnotherTestEvent(uint256 n);

    constructor() {
        emit TestEvent(1);
    }

    function f() external {
        emit TestEvent(2);
    }

    function g() external {
        emit AnotherTestEvent(1);
        this.f();
        emit AnotherTestEvent(2);
    }
}

// https://github.com/foundry-rs/foundry/issues/6643
contract Issue6643Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Counter public counter;

    event TestEvent(uint256 n);
    event AnotherTestEvent(uint256 n);

    function setUp() public {
        counter = new Counter();
    }

    function test_Bug1() public {
        // part1
        vm.expectEmit();
        emit TestEvent(1);
        new Counter();
        // part2
        vm.expectEmit();
        emit TestEvent(2);
        counter.f();
        // part3
        vm.expectEmit();
        emit AnotherTestEvent(1);
        vm.expectEmit();
        emit TestEvent(2);
        vm.expectEmit();
        emit AnotherTestEvent(2);
        counter.g();
    }

    function test_Bug2() public {
        vm.expectEmit();
        emit TestEvent(1);
        new Counter();
        vm.expectEmit();
        emit TestEvent(1);
        new Counter();
    }
}
