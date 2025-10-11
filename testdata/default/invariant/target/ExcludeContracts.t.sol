// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeContracts is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
        new Hello();
    }

    function excludeContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(hello);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
