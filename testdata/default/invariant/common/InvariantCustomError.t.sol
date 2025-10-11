// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.0;

import "utils/Test.sol";

contract ContractWithCustomError {
    error InvariantCustomError(uint256, string);

    function revertWithInvariantCustomError() external {
        revert InvariantCustomError(111, "custom");
    }
}

contract Handler is Test {
    ContractWithCustomError target;

    constructor() {
        target = new ContractWithCustomError();
    }

    function revertTarget() external {
        target.revertWithInvariantCustomError();
    }
}

contract InvariantCustomError is Test {
    Handler handler;

    function setUp() external {
        handler = new Handler();
    }

    function invariant_decode_error() public {}
}
