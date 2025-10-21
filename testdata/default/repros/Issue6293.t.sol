// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/6293
contract Issue6293Test is Test {
    constructor() {
        require(address(this).balance > 0);
        (bool success,) = payable(address(1)).call{value: 1}("");
        require(success, "call failed");
    }

    function test() public {
        assertGt(address(this).balance, 0);
    }
}
