// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Foo {}

contract GetNonceTest is Test {
    function testGetNonce() public {
        uint64 nonce1 = vm.getNonce(address(this));
        new Foo();
        new Foo();
        uint64 nonce2 = vm.getNonce(address(this));
        assertEq(nonce1 + 2, nonce2);
    }
}
