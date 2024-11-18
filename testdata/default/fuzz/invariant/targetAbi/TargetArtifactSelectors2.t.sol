// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";

struct FuzzArtifactSelector {
    string artifact;
    bytes4[] selectors;
}

contract Parent {
    bool public should_be_true = true;
    address public child;

    function change() public {
        child = msg.sender;
        should_be_true = false;
    }

    function create() public {
        new Child();
    }
}

contract Child {
    Parent parent;
    bool public changed = false;

    constructor() {
        parent = Parent(msg.sender);
    }

    function change_parent() public {
        parent.change();
    }

    function tracked_change_parent() public {
        parent.change();
    }
}

contract TargetArtifactSelectors2 is DSTest {
    Parent parent;

    function setUp() public {
        parent = new Parent();
    }

    function targetArtifactSelectors() public returns (FuzzArtifactSelector[] memory) {
        FuzzArtifactSelector[] memory targets = new FuzzArtifactSelector[](2);
        bytes4[] memory selectors_child = new bytes4[](1);

        selectors_child[0] = Child.change_parent.selector;
        targets[0] = FuzzArtifactSelector(
            "default/fuzz/invariant/targetAbi/TargetArtifactSelectors2.t.sol:Child", selectors_child
        );

        bytes4[] memory selectors_parent = new bytes4[](1);
        selectors_parent[0] = Parent.create.selector;
        targets[1] = FuzzArtifactSelector(
            "default/fuzz/invariant/targetAbi/TargetArtifactSelectors2.t.sol:Parent", selectors_parent
        );
        return targets;
    }

    function invariantShouldFail() public {
        if (!parent.should_be_true()) {
            require(!Child(address(parent.child())).changed(), "should have not happened");
        }
        require(parent.should_be_true() == true, "it's false");
    }
}
