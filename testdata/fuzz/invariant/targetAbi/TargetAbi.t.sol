// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ThirdHello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract TargetAbi is DSTest {
    Hello hello1;
    Hello hello2;
    ThirdHello hello3;

    function setUp() public {
        hello1 = new Hello();
        hello2 = new Hello();
        hello3 = new ThirdHello();
    }

    function targetAbis() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "fuzz/invariant/targetAbi/TargetAbi.t.sol:Hello";
        return abis;
    }

    function invariantTrueWorld() public {
        require(hello2.world() == true || hello1.world() == true || hello3.world() == true, "false world.");
    }

    function invariantFalseWorld() public {
        require(hello2.world() == true || hello1.world() == true, "false world.");
    }
}