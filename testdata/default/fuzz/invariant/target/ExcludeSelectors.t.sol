// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Hello {
    bool public world = false;

    function change() public {
        world = true;
    }

    function real_change() public {
        world = false;
    }
}

contract ExcludeSelectors is DSTest {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function excludeSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hello.change.selector;
        targets[0] = FuzzSelector(address(hello), selectors);
        return targets;
    }

    function invariantFalseWorld() public {
        require(hello.world() == false, "true world");
    }
}
