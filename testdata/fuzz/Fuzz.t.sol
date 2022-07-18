// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

interface Cheats {
    function toString(bytes32) external returns (string memory);
}

contract FuzzTest is DSTest {
    constructor() {
        emit log("constructor");
    }

    function setUp() public {
        emit log("setUp");
    }

    function testFailFuzz(uint8 x) public {
        emit log("testFailFuzz");
        require(x == 5, "should revert");
    }

    function testSuccessfulFuzz(uint128 a, uint128 b) public {
        emit log("testSuccessfulFuzz");
        assertEq(uint256(a) + uint256(b), uint256(a) + uint256(b));
    }

    function testToStringFuzz(bytes32 data) public {
        Cheats cheats = Cheats(HEVM_ADDRESS);
        cheats.toString(data);
    }
}
