// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        require(msg.sender == address(0xdeadbeef));
        world = false;
    }
}

contract TargetSenders is DSTest {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function targetSenders() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(0xdeadbeef);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world.");
    }
}
