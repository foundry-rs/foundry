pragma solidity ^0.8.8;

contract ForStatementComments {
    function bracedTrailingComment() external {
        uint256 x;
        for (uint256 i = 0; i < 10; ++i) { // step
            x++;
        }
    }

    function bracelessTrailingComment() external {
        uint256 x;
        for (uint256 i = 0; i < 10; ++i) x++; // step
    }

    function emptyBodyTrailingComment() external {
        for (uint256 i = 0; i < 10; ++i) { // step
        }
    }

    function leadingCommentUnaffected() external {
        // leading comment, must not move
        for (uint256 i = 0; i < 10; ++i) {
            i;
        }
    }

    function commentAfterHeaderNoCondNoNext() external {
        for (uint256 i = 0;;) // after header
        {}
    }

    function missingConditionTrailingComment() external {
        for (uint256 i = 0; ; ++i) { // c
            i;
        }
    }

    function missingIncrementTrailingComment() external {
        for (uint256 i = 0; i < 10;) { // c
            i;
        }
    }
}
