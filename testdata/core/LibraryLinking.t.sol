// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

library Lib {
    function plus100(uint256 a) public pure returns (uint256) {
        return a + 100;
    }
}

library NestedLib {
    function nestedPlus100Plus1(uint256 a) public pure returns (uint256) {
        return Lib.plus100(a) + 1;
    }
}

contract LibraryConsumer {
    function consume(uint256 a) public pure returns (uint256) {
        return Lib.plus100(a);
    }

    function consumeNested(uint256 a) public pure returns (uint256) {
        return NestedLib.nestedPlus100Plus1(a);
    }
}

contract LibraryLinkingTest is DSTest {
    LibraryConsumer consumer;

    function setUp() public {
        consumer = new LibraryConsumer();
    }

    function testDirect() public {
        assertEq(consumer.consume(1), 101, "library call failed");
    }

    function testNested() public {
        assertEq(consumer.consumeNested(1), 102, "nested library call failed");
    }
}
