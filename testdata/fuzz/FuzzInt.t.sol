// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

// See https://github.com/foundry-rs/foundry/pull/735 for context
contract FuzzNumbersTest is DSTest {
    function testPositive(int256) public {
        assertTrue(true);
    }

    function testNegativeHalf(int256 val) public {
        assertTrue(val < 2 ** 128 - 1);
    }

    function testNegative0(int256 val) public {
        assertTrue(val != 0);
    }

    function testNegative1(int256 val) public {
        assertTrue(val != -1);
    }

    function testNegative2(int256 val) public {
        assertTrue(val != 1);
    }

    function testNegativeMax0(int256 val) public {
        assertTrue(val != type(int256).max);
    }

    function testNegativeMax1(int256 val) public {
        assertTrue(val != type(int256).max - 2);
    }

    function testNegativeMin0(int256 val) public {
        assertTrue(val != type(int256).min);
    }


    function testNegativeMin1(int256 val) public {
        assertTrue(val != type(int256).min + 2);
    }

    function testEquality(int256 x, int256 y) public {
        int256 xy;

        unchecked {
            xy = x * y;
        }

        if ((x != 0 && xy / x != y)) return;

        assertEq(((xy - 1) / 1e18) + 1, (xy - 1) / (1e18 + 1));
    }
}
