// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeContracts is DSTest {
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
        require(hello.world() == true, "false world.");
    }
}