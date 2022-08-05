// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

struct FuzzAbiSelector {
    string contract_abi;
    bytes4[] selectors;
}

contract Hello {
    bool public world = true;

    function change() public {
        world = true;
    }
    
    function real_change() public {
        world = false;
    }
}

contract TargetAbiSelectors is DSTest {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function targetAbiSelectors() public returns (FuzzAbiSelector[] memory) {
        FuzzAbiSelector[] memory targets = new FuzzAbiSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hello.change.selector;
        targets[0] = FuzzAbiSelector("fuzz/invariant/targetAbi/TargetAbiSelectors.t.sol:Hello", selectors);
        return targets;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world.");
    }
}