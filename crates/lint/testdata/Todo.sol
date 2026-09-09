//@compile-flags: --only-lint todo-comment

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract Todo {
    // see TODO_ITEMS_LIMIT for the cap
    uint256 constant TODO_ITEM_LIST = 10;

    uint8 x = 1; //TODO: validate this

    // ToDo: implement access control
    function unfinished() public {}

    // todo-list should not be treated as an unresolved marker if whole words are required
    function todolist() public {}

    // Read todo.md before editing this contract
    function todoMarkdown() public {}

    // FixMe: this is broken
    function buggy() public {}

    // TODO. This should still be treated as an unresolved marker
    function sentenceMarker() public {}

    /*TODO(alice): this one should be fixed */
    function buggy2() public {}

    /* ToDo: revisit this math */
    function math() public {}

    /// ToDo: document this properly
    function documented() public {}

    // check other tickets in todo list
    function todoInBetween() public {}

    // FIXME first, TODO second, and fixme third
    function combined() public {}

    // TODO: first, todo: second, FIXME: third, fixme: fourth
    function deduplicated() public {}

    // a perfectly normal comment, no markers
    function clean() public {}

    // forge-lint: disable-next-line(todo-comment)
    // TODO: this marker is intentionally suppressed
    function suppressed() public {}

    // TODO
    function bareMarker() public {}

    // TODO implement access control
    function bareTodoSentence() public {}

    // FIXME this check is wrong
    function bareFixmeSentence() public {}

    // check this later TODO
    function trailingBare() public {}

    /*
        TODO: this is a block comment with a marker, should be detected
    */
    function blockComment() public {}

    /* Context for this block comment.
     * TODO implement the starred block-comment work
     */
    function starredBareBlockComment() public {}

    /* Context for this block comment.
       FIXME implement the plain block-comment work
    */
    function plainBareBlockComment() public {}

    /// @notice This function does something important
    /// @dev TODO: implement the actual logic here
    function natSpec() public pure returns (uint256) {
        return 42;
    }

    ///@dev TODO: compact NatSpec should also be detected
    function compactNatSpec() public {}

    /// @dev TODO implement the actual logic here
    function bareNatSpec() public {}

    /**
     * @dev TODO implement the block NatSpec logic here
     */
    function bareBlockNatSpec() public {}

    function noFalsePositiveInStrings() public pure returns (string memory) {
        // The marker below is inside a string literal, must NOT fire:
        return "this TODO is just data, not a comment";
    }
}
