// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract ResetNonce is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Foo public fooContract;
    address barEOA;

    function setUp() public {
        fooContract = new Foo();
        barEOA = address(0x42);
    }

    function testResetNonceContract() public {
        cheats.setNonce(address(fooContract), 10);

        // makes sure working correctly after mutating nonce.
        fooContract.f();
        assertEq(cheats.getNonce(address(fooContract)), 10);
        fooContract.f();

        // now make sure that it is reset after calling the cheatcode.
        cheats.resetNonce(address(fooContract));
        assertEq(cheats.getNonce(address(fooContract)), 1);
        fooContract.f();
    }

    function testResetNonceEOA() public {
        cheats.setNonce(address(barEOA), 10);
        assertEq(cheats.getNonce(address(barEOA)), 10);
        cheats.resetNonce(address(barEOA));
        assertEq(cheats.getNonce(address(barEOA)), 0);
    }
}
