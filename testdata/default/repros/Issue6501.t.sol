// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../logs/console.sol";

// https://github.com/foundry-rs/foundry/issues/6501
contract Issue6501Test is DSTest {
    function test_hhLogs() public {
        console.log("a");
        console.log(uint256(1));
        console.log("b", uint256(2));
    }
}
