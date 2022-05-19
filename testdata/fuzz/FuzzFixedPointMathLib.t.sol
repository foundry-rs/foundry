// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import {DSTestPlus} from "../lib/solmate/src/test/utils/DSTestPlus.sol";
import {FixedPointMathLib} from "../lib/solmate/src/utils/FixedPointMathLib.sol";

contract FuzzFixedPointMathLibTest is DSTestPlus {
    function testMulWadDown() public {
        assertEq(FixedPointMathLib.mulWadDown(2.5e18, 0.5e18), 1.25e18);
        assertEq(FixedPointMathLib.mulWadDown(3e18, 1e18), 3e18);
        assertEq(FixedPointMathLib.mulWadDown(369, 271), 0);
    }
}