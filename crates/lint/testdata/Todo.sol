//@compile-flags: --only-lint todo

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract Todo {
    // ToDo: implement access control
    function unfinished() public {}

    // FixMe this is broken
    function buggy() public {}

    // fixme this is broken too
    function buggy2() public {}

    /* ToDo: revisit this math */
    function math() public {}

    /// ToDo: document this properly
    function documented() public {}

    // FIXME first, TODO second, and fixme third
    function combined() public {}

    // a perfectly normal comment, no markers
    function clean() public {}

    /*
        TODO: this is a block comment with a marker, should be detected
    */
    function blockComment() public {}

    /// @notice This function does something important
    /// @dev TODO: implement the actual logic here
    function natSpec() public pure returns (uint256) {
        return 42;
    }

    function noFalsePositiveInStrings() public pure returns (string memory) {
        // The marker below is inside a string literal, must NOT fire:
        return "this TODO is just data, not a comment";
    }
}
