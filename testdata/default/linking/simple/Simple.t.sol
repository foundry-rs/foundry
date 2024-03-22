// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

// Linking scenario: contract with one library

library Lib {
    function plus100(uint256 a) public pure returns (uint256) {
        return a + 100;
    }
}

contract LibraryConsumer {
    function consume(uint256 a) public pure returns (uint256) {
        return Lib.plus100(a);
    }
}

contract SimpleLibraryLinkingTest is DSTest {
    LibraryConsumer consumer;

    function setUp() public {
        consumer = new LibraryConsumer();
    }

    function testCall() public {
        assertEq(consumer.consume(1), 101, "library call failed");
    }
}
