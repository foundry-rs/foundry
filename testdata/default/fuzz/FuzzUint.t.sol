// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

// See https://github.com/foundry-rs/foundry/pull/735 for context
contract FuzzNumbersTest is DSTest {
    function testPositive(uint256) public {
        assertTrue(true);
    }

    function testNegativeHalf(uint256 val) public {
        assertTrue(val < 2 ** 128 - 1);
    }

    function testNegative0(uint256 val) public {
        assertTrue(val != 0);
    }

    function testNegative2(uint256 val) public {
        assertTrue(val != 2);
    }

    function testNegative2Max(uint256 val) public {
        assertTrue(val != type(uint256).max - 2);
    }

    function testNegativeMax(uint256 val) public {
        assertTrue(val != type(uint256).max);
    }

    function testEquality(uint256 x, uint256 y) public {
        uint256 xy;

        unchecked {
            xy = x * y;
        }

        if ((x != 0 && xy / x != y)) {
            return;
        }

        assertEq(((xy - 1) / 1e18) + 1, (xy - 1) / (1e18 + 1));
    }
}
