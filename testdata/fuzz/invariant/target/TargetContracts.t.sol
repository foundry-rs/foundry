// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract TargetContracts is DSTest {
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
        require(hello2.world() == true, "false world.");
    }
}
