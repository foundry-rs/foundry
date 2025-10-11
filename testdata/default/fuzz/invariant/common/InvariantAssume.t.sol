// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.0;

import "utils/Test.sol";

contract Handler is Test {
    function doSomething(uint256 param) public {
        vm.assume(param == 0);
    }
}

contract InvariantAssume is Test {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function invariant_dummy() public {}
}
