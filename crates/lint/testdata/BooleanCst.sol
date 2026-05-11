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
        return true;
    }

    function takesBool(bool value) internal pure {}
}
