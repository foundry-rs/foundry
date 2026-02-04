// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/1543
contract BeforeTestSelfDestructTest is Test {
    uint256 a;
    uint256 b;

    function beforeTestSetup(bytes4 testSelector) public pure returns (bytes[] memory beforeTestCalldata) {
        if (testSelector == this.testA.selector) {
            beforeTestCalldata = new bytes[](3);
            beforeTestCalldata[0] = abi.encodePacked(this.testA.selector);
            beforeTestCalldata[1] = abi.encodePacked(this.testA.selector);
            beforeTestCalldata[2] = abi.encodePacked(this.testA.selector);
        }

        if (testSelector == this.testB.selector) {
            beforeTestCalldata = new bytes[](1);
            beforeTestCalldata[0] = abi.encodePacked(this.setB.selector);
        }

        if (testSelector == this.testC.selector) {
            beforeTestCalldata = new bytes[](2);
            beforeTestCalldata[0] = abi.encodePacked(this.testA.selector);
            beforeTestCalldata[1] = abi.encodeWithSignature("setBWithValue(uint256)", 111);
        }
    }

    function testA() public {
        assertLe(a, 3);
        a += 1;
    }

    function testSimpleA() public {
        assertEq(a, 0);
    }

    function setB() public {
        b = 100;
    }

    function testB() public {
        assertEq(b, 100);
    }

    function setBWithValue(uint256 value) public {
        b = value;
    }

    function testC(uint256 h) public {
        assertEq(a, 1);
        assertEq(b, 111);
    }
}
