// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Source {
    uint256 public a;
    address public b;
    uint256[3] public c;
    bool public d;

    constructor() {
        a = 100;
        b = address(111);
        c[0] = 222;
        c[1] = 333;
        c[2] = 444;
        d = true;
    }
}

contract CloneAccountTest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);

    address clone = address(777);

    function setUp() public {
        Source src = new Source();
        vm.deal(address(src), 0.123 ether);
        vm.cloneAccount(address(src), clone);
    }

    function test_clone_account() public {
        // Check clone balance.
        assertEq(clone.balance, 0.123 ether);
        // Check clone storage.
        assertEq(Source(clone).a(), 100);
        assertEq(Source(clone).b(), address(111));
        assertEq(Source(clone).c(0), 222);
        assertEq(Source(clone).c(1), 333);
        assertEq(Source(clone).c(2), 444);
        assertEq(Source(clone).d(), true);
    }
}
