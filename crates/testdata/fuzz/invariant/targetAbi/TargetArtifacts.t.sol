// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract Targeted {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hello {
    bool public world = true;

    function no_change() public {}
}

contract TargetArtifacts is DSTest {
    Targeted target1;
    Targeted target2;
    Hello hello;

    function setUp() public {
        target1 = new Targeted();
        target2 = new Targeted();
        hello = new Hello();
    }

    function targetArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "fuzz/invariant/targetAbi/TargetArtifacts.t.sol:Targeted";
        return abis;
    }

    function invariantShouldPass() public {
        require(target2.world() == true || target1.world() == true || hello.world() == true, "false world.");
    }

    function invariantShouldFail() public {
        require(target2.world() == true || target1.world() == true, "false world.");
    }
}
