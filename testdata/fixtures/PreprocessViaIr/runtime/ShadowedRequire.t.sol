// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.24 <0.9.0;

import {Empty} from "../src/Targets.sol";

function require(bool) pure {
    revert("shadowed require");
}

contract ShadowedRequireTest {
    function test_shadowed_require() public {
        Empty target = new Empty();
        if (target.caller() != address(this) || target.deployedAt() != address(target)) {
            revert("deployment state");
        }
    }
}
