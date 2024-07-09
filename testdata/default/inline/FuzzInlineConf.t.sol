// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract FuzzInlineConf is DSTest {
    /**
     * forge-config: default.fuzz.runs = 1024
     * forge-config: default.fuzz.max-test-rejects = 500
     */
    function testInlineConfFuzz(uint8 x) public {
        require(true, "this is not going to revert");
    }
}

/// forge-config: default.fuzz.runs = 10
contract FuzzInlineConf2 is DSTest {
    /// forge-config: default.fuzz.runs = 1
    function testInlineConfFuzz1(uint8 x) public {
        require(true, "this is not going to revert");
    }

    function testInlineConfFuzz2(uint8 x) public {
        require(true, "this is not going to revert");
    }
}
