// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

struct FuzzAbiSelector {
    string contract_abi;
    bytes4[] selectors;
}

contract Hi {
    bool public world = true;

    function no_change() public {
        world = true;
    }

    function changee() public {
        world = false;
    }
}

contract TargetArtifactSelectors is DSTest {
    Hi hello;

    function setUp() public {
        hello = new Hi();
    }

    function targetArtifactSelectors() public returns (FuzzAbiSelector[] memory) {
        FuzzAbiSelector[] memory targets = new FuzzAbiSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hi.no_change.selector;
        targets[0] = FuzzAbiSelector("fuzz/invariant/targetAbi/TargetArtifactSelectors.t.sol:Hi", selectors);
        return targets;
    }

    function invariantShouldPass() public {
        require(hello.world() == true, "false world");
    }
}
