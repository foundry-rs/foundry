// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Counter {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function setAnotherNumber(uint256 newNumber) public {
        anotherNumber = newNumber;
    }

    function increment() public {
        number++;
    }
}

contract Issue10552Test is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    Counter public counter;
    uint256 mainnetId;
    uint256 opId;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(10);
        VM.makePersistent(address(counter));

        mainnetId = VM.createFork("mainnet");
        opId = VM.createFork("optimism");

        VM.selectFork(mainnetId);
        counter.setNumber(100);
        counter.increment();
        assertEq(counter.number(), 101);

        counter.increment();
        assertEq(counter.number(), 102);
    }

    function test_change_fork_states() public {
        VM.selectFork(opId);
        counter.increment();
        // should account state changes from mainnet fork
        // without fix for <https://github.com/foundry-rs/foundry/issues/10552> this test was failing with 11 (initial setNumber(10) + one increment) != 103
        assertEq(counter.number(), 103);
        counter.setAnotherNumber(11);
        assertEq(counter.anotherNumber(), 11);

        VM.selectFork(mainnetId);
        counter.increment();
        assertEq(counter.number(), 104);
        assertEq(counter.anotherNumber(), 11);
    }
}
