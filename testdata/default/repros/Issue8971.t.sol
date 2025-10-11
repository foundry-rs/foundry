// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Counter {
    uint256 public number;

    function increment() public {
        number++;
    }
}

/// @notice Test is mostly related to --isolate. Ensures that state is not affected by reverted
/// call to handler.
contract Handler {
    bool public locked;
    Counter public counter = new Counter();

    function doNothing() public {}

    function doSomething() public {
        locked = true;
        counter.increment();
        this.doRevert();
    }

    function doRevert() public {
        revert();
    }
}

/// forge-config: default.isolate = true
contract Invariant is Test {
    Handler h;

    function setUp() public {
        h = new Handler();
    }

    function targetContracts() public view returns (address[] memory contracts) {
        contracts = new address[](1);
        contracts[0] = address(h);
    }

    function invariant_unchanged() public {
        assertEq(h.locked(), false);
        assertEq(h.counter().number(), 0);
    }
}
