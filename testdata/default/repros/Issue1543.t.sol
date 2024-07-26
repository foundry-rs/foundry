// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract SelfDestructor {
    function kill() external {
        selfdestruct(payable(msg.sender));
    }
}

// https://github.com/foundry-rs/foundry/issues/1543
contract Issue1543Test is DSTest {
    SelfDestructor killer;
    uint256 a;
    uint256 b;

    function setUp() public {
        killer = new SelfDestructor();
    }

    function beforeTestSetup(bytes4 testSelector) public pure returns (bytes[] memory beforeTestCalldata) {
        if (testSelector == this.testKill.selector) {
            beforeTestCalldata = new bytes[](1);
            beforeTestCalldata[0] = abi.encodePacked(this.kill_contract.selector);
        }

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

    function kill_contract() external {
        uint256 killer_size = getSize(address(killer));
        require(killer_size == 106);
        killer.kill();
    }

    function testKill() public view {
        uint256 killer_size = getSize(address(killer));
        require(killer_size == 0);
    }

    function getSize(address c) public view returns (uint32) {
        uint32 size;
        assembly {
            size := extcodesize(c)
        }
        return size;
    }

    function testA() public {
        require(a <= 3);
        a += 1;
    }

    function testSimpleA() public view {
        require(a == 0);
    }

    function setB() public {
        b = 100;
    }

    function testB() public {
        require(b == 100);
    }

    function setBWithValue(uint256 value) public {
        b = value;
    }

    function testC(uint256 h) public {
        assertEq(a, 1);
        assertEq(b, 111);
    }
}
