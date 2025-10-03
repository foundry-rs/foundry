// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract FuzzTest is Test {
    constructor() {
        emit log("constructor");
    }

    function setUp() public {
        emit log("setUp");
    }

    function testShouldFailFuzz(uint8 x) public {
        emit log("testFailFuzz");
        require(x > 128, "should revert");
    }

    function testSuccessfulFuzz(uint128 a, uint128 b) public {
        emit log("testSuccessfulFuzz");
        assertEq(uint256(a) + uint256(b), uint256(a) + uint256(b));
    }

    function testToStringFuzz(bytes32 data) public {
        vm.toString(data);
    }
}
