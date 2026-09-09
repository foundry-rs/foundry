//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract BooleanEqual {
    function check(bool enabled, bool paused, bool ready, bool done) public pure {
        if (enabled == true) {} //~NOTE: boolean comparison to a constant is redundant
        if (paused == false) {} //~NOTE: boolean comparison to a constant is redundant
        if (true != ready) {} //~NOTE: boolean comparison to a constant is redundant
        while (done != false) { //~NOTE: boolean comparison to a constant is redundant
            break;
        }
        for (; enabled == true && paused != false;) {
            //~^NOTE: boolean comparison to a constant is redundant
            //~|NOTE: boolean comparison to a constant is redundant
            break;
        }
    }

    function returnedComparison(bool enabled) public pure returns (bool) {
        return enabled == true; //~NOTE: boolean comparison to a constant is redundant
    }
}
