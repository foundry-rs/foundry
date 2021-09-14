// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.1;

contract DsTestMini {
    bool public failed;

    function fail() private {
        failed = true;
    }

    function assertEq(uint a, uint b) internal {
        if (a != b) {
            fail();
        }
    }
}

contract FooTest is DsTestMini {
    uint256 x;

    function setUp() public {
        x = 1;
    }

    function testX() public {
        require(x == 1, "x is not one");
    }

    function testFailX() public {
        assertEq(x, 2);
    }
}
