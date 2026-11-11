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

    // An underfunded create fails before it increments the creator's nonce, after its transaction
    // paid the intrinsic gas.
    function testUnderfundedCreateKeepsNonce() public {
        uint64 nonce = vm.getNonce(address(this));
        uint256 value = address(this).balance + 1;
        address created;
        uint256 gasUsed = gasleft();
        assembly {
            created := create(value, 0, 0)
        }
        gasUsed -= gasleft();
        assertEq(created, address(0));
        assertEq(vm.getNonce(address(this)), nonce);
        // The CREATE opcode and the create transaction's intrinsic gas.
        assertGe(gasUsed, 32_000 + 53_000);
    }
}
