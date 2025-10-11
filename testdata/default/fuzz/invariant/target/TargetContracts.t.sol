// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract TargetContracts is Test {
    Hello hello1;
    Hello hello2;

    function setUp() public {
        hello1 = new Hello();
        hello2 = new Hello();
    }

    function targetContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(hello1);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello2.world() == true, "false world");
    }
}
