// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Todo {
    // TODO: implement access control
    function unfinished() public {}

    // FIXME this is broken
    function buggy() public {}

    /* TODO: revisit this math */
    function math() public {}

    /// TODO: document this properly
    function documented() public {}

    // a perfectly normal comment, no markers
    function clean() public {}

    function noFalsePositiveInStrings() public pure returns (string memory) {
        // The marker below is inside a string literal, must NOT fire:
        return "this TODO is just data, not a comment";
    }
}
