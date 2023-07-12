// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";

// Linking scenario: contract has many dependencies, some of which appear to the linker
// more than once.
//
// Each library should only have its address computed once

library A {
    function increment(uint256 number, uint256 number2) external pure returns (uint256) {
        return number + number2;
    }
}

library B {
    function subtract(uint256 number) external pure returns (uint256) {
        return number - 1;
    }
}

library C {
    function double(uint256 number) external pure returns (uint256) {
        return A.increment(number, 0) + A.increment(number, 0);
    }
}

library D {
    function half(uint256 number) external pure returns (uint256) {
        return number / 2;
    }

    function sub2(uint256 number) external pure returns (uint256) {
        return B.subtract(number);
    }
}

library E {
    function pow(uint256 number, uint256 exponent)
        external
        pure
        returns (uint256)
    {
        return number**exponent;
    }

    function quadruple(uint256 number) external pure returns (uint256) {
        return C.double(number) + C.double(number);
    }
}

contract LibraryConsumer {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }

    function add(uint256 num) external returns (uint256) {
        number = num;
        return A.increment(num, 1);
    }

    function sub(uint256 num) external returns (uint256) {
        number = num;
        return B.subtract(num);
    }

    function mul(uint256 num) external returns (uint256) {
        number = num;
        return C.double(num);
    }

    function div(uint256 num) external returns (uint256) {
        number = num;
        return D.half(num);
    }

    function pow(uint256 num, uint256 exponent) external returns (uint256) {
        number = num;
        return E.pow(num, exponent);
    }

    function sub2(uint256 num) external returns (uint256) {
        number = num;
        return D.sub2(num);
    }

    function quadruple(uint256 num) external returns (uint256) {
        number = num;
        return E.quadruple(num);
    }
}

contract DuplicateLibraryLinkingTest is DSTest {
    LibraryConsumer consumer;

    function setUp() public {
        consumer = new LibraryConsumer();
    }

    function testA() public {
        assertEq(consumer.add(1), 2, "library call failed");
    }

    function testB() public {
        consumer.setNumber(1);
        assertEq(consumer.sub(1), 0, "library call failed");
    }

    function testC() public {
        consumer.setNumber(2);
        assertEq(consumer.mul(2), 4, "library call failed");
    }

    function testD() public {
        consumer.setNumber(2);
        assertEq(consumer.div(2), 1, "library call failed");
    }

    function testE() public {
        consumer.setNumber(2);
        assertEq(consumer.quadruple(2), 8, "library call failed");
    }
}
