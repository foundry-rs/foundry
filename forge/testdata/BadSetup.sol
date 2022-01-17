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

contract SetupTest is DsTestMini {
    function setUp() public {
        T t = new T(10);
    }

    function testSetupBad() public {
    }

    function testSetupBad2() public {
    }
}


contract T {
    constructor(uint256 a) {
        uint256 b = a - 100;
    }
}