// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3347
contract Issue3347Test is DSTest {
    event log2(uint256, uint256);

    function test() public {
        emit log2(1, 2);
    }
}
