// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.11;

// a library that needs to be linked with another library
library LibTestNested {
    enum TestEnum2 {
        A,
        B,
        C
    }

    function foobar(TestEnum2 test) public view returns (uint256) {
    	return LibTest.foobar(101);
    }
}

// a library
library LibTest {
    function foobar(uint256 a) public view returns (uint256) {
    	return a * 100;
    }
}


// a contract that uses 2 linked libraries
contract Main {
    function foo() public returns (uint256) {
        return LibTest.foobar(1);
    }

    function bar() public returns (uint256) {
        return LibTestNested.foobar(LibTestNested.TestEnum2(0));
    }
}

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


contract LibLinkingTest is DsTestMini {
    Main main;
    function setUp() public {
        main = new Main();
    }

    function testCall() public {
        assertEq(100, main.foo());
    }

    function testCall2() public {
        assertEq(10100, main.bar());
    }
}
