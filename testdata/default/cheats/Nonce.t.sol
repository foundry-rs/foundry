// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Counter {
    uint256 public count;

    function increment() public {
        count += 1;
    }
}

/// forge-config: default.isolate = true
contract NonceIsolatedTest is Test {
    function testIncrementNonce() public {
        address bob = address(14);
        vm.startPrank(bob);
        Counter counter = new Counter();
        assertEq(vm.getNonce(bob), 1);
        counter.increment();
        assertEq(vm.getNonce(bob), 2);
        new Counter();
        assertEq(vm.getNonce(bob), 3);
        counter.increment();
        assertEq(vm.getNonce(bob), 4);
    }
}
