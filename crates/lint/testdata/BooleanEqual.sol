//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract BooleanEqual {
    function check(bool enabled, bool paused, bool ready, bool done) public pure {
        if (enabled == true) {} //~NOTE: boolean comparison to a constant can be simplified
        if (paused == false) {} //~NOTE: boolean comparison to a constant can be simplified
        if (true != ready) {} //~NOTE: boolean comparison to a constant can be simplified
        while (done != false) { //~NOTE: boolean comparison to a constant can be simplified
            break;
        }
        for (; enabled == true && paused != false;) {
            //~^NOTE: boolean comparison to a constant can be simplified
            //~|NOTE: boolean comparison to a constant can be simplified
            break;
        }
    }

    function returnedComparison(bool enabled) public pure returns (bool) {
        return enabled == true; //~NOTE: boolean comparison to a constant can be simplified
    }
}
