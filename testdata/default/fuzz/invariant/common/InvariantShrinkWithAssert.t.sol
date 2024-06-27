// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }

    function decrement() public {
        number--;
    }

    function double() public {
        number *= 2;
    }

    function half() public {
        number /= 2;
    }

    function triple() public {
        number *= 3;
    }

    function third() public {
        number /= 3;
    }

    function quadruple() public {
        number *= 4;
    }

    function quarter() public {
        number /= 4;
    }
}

contract Handler is DSTest {
    Counter public counter;

    constructor(Counter _counter) {
        counter = _counter;
        counter.setNumber(0);
    }

    function increment() public {
        counter.increment();
    }

    function setNumber(uint256 x) public {
        counter.setNumber(x);
    }
}

contract InvariantShrinkWithAssert is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Counter public counter;
    Handler handler;

    function setUp() public {
        counter = new Counter();
        handler = new Handler(counter);
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.increment.selector;
        selectors[1] = handler.setNumber.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function invariant_with_assert() public {
        assertTrue(counter.number() != 3, "wrong counter");
    }
}

contract InvariantShrinkWithRequire is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Counter public counter;
    Handler handler;

    function setUp() public {
        counter = new Counter();
        handler = new Handler(counter);
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.increment.selector;
        selectors[1] = handler.setNumber.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function invariant_with_require() public {
        require(counter.number() != 3, "wrong counter");
    }
}
