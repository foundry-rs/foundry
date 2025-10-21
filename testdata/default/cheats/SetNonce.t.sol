// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract SetNonceTest is Test {
    Foo public foo;

    function setUp() public {
        foo = new Foo();
    }

    function testSetNonce() public {
        vm.setNonce(address(foo), 10);
        // makes sure working correctly after mutating nonce.
        foo.f();
        assertEq(vm.getNonce(address(foo)), 10);
        foo.f();
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testRevertIfInvalidNonce() public {
        vm.setNonce(address(foo), 10);
        // set lower nonce should fail
        vm.expectRevert(
            "vm.setNonce: new nonce (5) must be strictly equal to or higher than the account's current nonce (10)"
        );
        vm.setNonce(address(foo), 5);
    }
}
