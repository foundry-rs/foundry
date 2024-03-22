// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract Hello {
    address seed_address = address(0xdeadbeef);
    bool public world = true;

    function changeBeef() public {
        require(msg.sender == address(0xdeadbeef));
        world = false;
    }

    // address(0) should be automatically excluded
    function change0() public {
        require(msg.sender == address(0));
        world = false;
    }
}

contract ExcludeSenders is DSTest {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function excludeSenders() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(0xdeadbeef);
        return addrs;
    }

    // Tests clashing. Exclusion takes priority.
    function targetSenders() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(0xdeadbeef);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
