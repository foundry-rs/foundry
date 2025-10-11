// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// Will get automatically excluded. Otherwise it would throw error.
contract NoMutFunctions {
    function no_change() public pure {}
}

contract Excluded {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeArtifacts is Test {
    Excluded excluded;

    function setUp() public {
        excluded = new Excluded();
        new Hello();
        new NoMutFunctions();
    }

    function excludeArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "default/fuzz/invariant/targetAbi/ExcludeArtifacts.t.sol:Excluded";
        return abis;
    }

    function invariantShouldPass() public {
        require(excluded.world() == true, "false world");
    }
}
