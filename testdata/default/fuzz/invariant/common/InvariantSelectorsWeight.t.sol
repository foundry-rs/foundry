// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";

contract HandlerWithOneSelector {
    uint256 public hit1;

    function selector1() external {
        hit1 += 1;
    }
}

contract HandlerWithFiveSelectors {
    uint256 public hit2;
    uint256 public hit3;
    uint256 public hit4;
    uint256 public hit5;
    uint256 public hit6;

    function selector2() external {
        hit2 += 1;
    }

    function selector3() external {
        hit3 += 1;
    }

    function selector4() external {
        hit4 += 1;
    }

    function selector5() external {
        hit5 += 1;
    }

    function selector6() external {
        hit6 += 1;
    }
}

contract HandlerWithFourSelectors {
    uint256 public hit7;
    uint256 public hit8;
    uint256 public hit9;
    uint256 public hit10;

    function selector7() external {
        hit7 += 1;
    }

    function selector8() external {
        hit8 += 1;
    }

    function selector9() external {
        hit9 += 1;
    }

    function selector10() external {
        hit10 += 1;
    }
}

contract InvariantSelectorsWeightTest is DSTest {
    HandlerWithOneSelector handlerOne;
    HandlerWithFiveSelectors handlerTwo;
    HandlerWithFourSelectors handlerThree;

    function setUp() public {
        handlerOne = new HandlerWithOneSelector();
        handlerTwo = new HandlerWithFiveSelectors();
        handlerThree = new HandlerWithFourSelectors();
    }

    function afterInvariant() public {
        // selector hits before and after https://github.com/foundry-rs/foundry/issues/2986
        // hit1: 11 | hit2: 4 | hit3: 0 | hit4: 0 | hit5: 4 | hit6: 1 | hit7: 2 | hit8: 2 | hit9: 2 | hit10: 4
        // hit1:  2 | hit2: 5 | hit3: 4 | hit4: 5 | hit5: 3 | hit6: 1 | hit7: 4 | hit8: 1 | hit9: 1 | hit10: 4

        uint256 hit1 = handlerOne.hit1();
        uint256 hit2 = handlerTwo.hit2();
        uint256 hit3 = handlerTwo.hit3();
        uint256 hit4 = handlerTwo.hit4();
        uint256 hit5 = handlerTwo.hit5();
        uint256 hit6 = handlerTwo.hit6();
        uint256 hit7 = handlerThree.hit7();
        uint256 hit8 = handlerThree.hit8();
        uint256 hit9 = handlerThree.hit9();
        uint256 hit10 = handlerThree.hit10();

        require(
            hit1 > 0 && hit2 > 0 && hit3 > 0 && hit4 > 0 && hit5 > 0 && hit6 > 0 && hit7 > 0 && hit8 > 0 && hit9 > 0
                && hit10 > 0
        );
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 30
    function invariant_selectors_weight() public view {}
}
