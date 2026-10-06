// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract BooleanCst {
    function check(bool flag) public pure returns (bool) {
        if (false) {} //~WARN: misuse of a boolean constant
        if (flag || true) {} //~WARN: misuse of a boolean constant
        if (flag ? true : false) {}
        //~^WARN: misuse of a boolean constant
        //~|WARN: misuse of a boolean constant
        while (true) {
            break;
        }

        bool assigned = true;
        return assigned && false; //~WARN: misuse of a boolean constant
    }

    function allowedBareConstants(bool flag) public pure returns (bool) {
        takesBool(true);
        takesBool(false);
        return true;
    }

    // A tuple only groups, so returning a literal inside one is as fine as
    // returning it bare. This is how a (bool, T) result is written.
    function allowedInTuple(bytes memory input) public pure returns (bool, uint256) {
        if (input.length == 0) return (false, 0);
        return (true, input.length);
    }

    // Parentheses do not make a condition acceptable.
    function stillFlaggedWhenGrouped(bool flag) public pure returns (bool) {
        if ((true)) {} //~WARN: misuse of a boolean constant
        return flag || (false); //~WARN: misuse of a boolean constant
    }

    function takesBool(bool value) internal pure {
        value;
    }
}
