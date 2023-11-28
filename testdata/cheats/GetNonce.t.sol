// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract Foo {}

contract GetNonceTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetNonce() public {
        uint64 nonce1 = vm.getNonce(address(this));
        new Foo();
        new Foo();
        uint64 nonce2 = vm.getNonce(address(this));
        assertEq(nonce1 + 2, nonce2);
    }
}
