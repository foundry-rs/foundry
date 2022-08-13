// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

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

contract ExcludeArtifacts is DSTest {
    Excluded excluded;

    function setUp() public {
        excluded = new Excluded();
        new Hello();
    }

    function excludeArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "fuzz/invariant/targetAbi/ExcludeArtifacts.t.sol:Excluded";
        return abis;
    }

    function invariantShouldPass() public {
        require(excluded.world() == true, "false world.");
    }
}
