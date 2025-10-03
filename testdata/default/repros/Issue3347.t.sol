// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3347
contract Issue3347Test is Test {
    event log2(uint256, uint256);

    function test() public {
        emit log2(1, 2);
    }
}
