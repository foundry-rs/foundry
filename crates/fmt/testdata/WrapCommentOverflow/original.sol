pragma solidity ^0.8.13;

contract WrapCommentOverflowTest {
    /// @notice This is a very long single-line comment that should demonstrate strategic overflow wrapping behavior without creating orphaned words
    function singleLineOverflow() public {}
    
    /// @notice Calculates the amount that the sender would be refunded if the stream were canceled, denominated in units of the token's decimals.
    function originalGitHubIssue() public {}
    
    /// @notice Short comment that fits on one line
    function singleLineNoWrap() public {}
    
    /// @notice This is a notice section that is quite long and should wrap nicely
    /// @param value This parameter description should remain separate from the notice above
    /// @return result The return value description should also stay separate
    function natspecBoundaries(uint256 value) public returns (uint256 result) {}
    
    /// @notice Another example with multiple sections
    /// @dev Implementation details that are separate from notice
    /// @param amount Should not merge with dev section above
    function multipleSections(uint256 amount) public {}
    
    /// @notice Function with markdown list that should preserve structure:
    /// - First item in the list should stay as a list item
    /// - Second item should also remain properly formatted
    /// - Third item completes the list structure
    function markdownList() public {}
    
    /// @notice Another markdown example:
    /// 1. Numbered list item one
    /// 2. Numbered list item two that is longer
    /// 3. Final numbered item
    function numberedList() public {}
    
    /// @notice Block quote example:
    /// > This is a block quote that should remain intact
    /// > Second line of the block quote
    function blockQuote() public {}
    
    /// @notice First paragraph of documentation
    ///
    /// Second paragraph should remain separate due to empty line above
    function emptyLineSeparation() public {}
}
