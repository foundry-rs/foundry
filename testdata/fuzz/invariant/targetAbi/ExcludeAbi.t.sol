// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hi {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeAbi is DSTest {
    Hello hello;

    function setUp() public {
        hello = new Hello();
        new Hi();
    }

    function excludeAbis() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "fuzz/invariant/targetAbi/ExcludeAbi.t.sol:Hello";
        return abis;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world.");
    }
}