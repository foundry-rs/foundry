// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Vm.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract ResetNonce is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Foo public fooContract;
    address barEOA;

    function setUp() public {
        fooContract = new Foo();
        barEOA = address(0x42);
    }

    function testResetNonceContract() public {
        vm.setNonce(address(fooContract), 10);

        // makes sure working correctly after mutating nonce.
        fooContract.f();
        assertEq(vm.getNonce(address(fooContract)), 10);
        fooContract.f();

        // now make sure that it is reset after calling the cheatcode.
        vm.resetNonce(address(fooContract));
        assertEq(vm.getNonce(address(fooContract)), 1);
        fooContract.f();
    }

    function testResetNonceEOA() public {
        vm.setNonce(address(barEOA), 10);
        assertEq(vm.getNonce(address(barEOA)), 10);
        vm.resetNonce(address(barEOA));
        assertEq(vm.getNonce(address(barEOA)), 0);
    }
}
