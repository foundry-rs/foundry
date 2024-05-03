// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

struct FuzzArtifactSelector {
    string artifact;
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

    function targetArtifactSelectors() public returns (FuzzArtifactSelector[] memory) {
        FuzzArtifactSelector[] memory targets = new FuzzArtifactSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hi.no_change.selector;
        targets[0] =
            FuzzArtifactSelector("default/fuzz/invariant/targetAbi/TargetArtifactSelectors.t.sol:Hi", selectors);
        return targets;
    }

    function invariantShouldPass() public {
        require(hello.world() == true, "false world");
    }
}
