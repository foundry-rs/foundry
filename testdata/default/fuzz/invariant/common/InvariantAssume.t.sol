// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.0;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Handler is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function doSomething(uint256 param) public {
        vm.assume(param == 0);
    }
}

contract InvariantAssume is DSTest {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function invariant_dummy() public {}
}
