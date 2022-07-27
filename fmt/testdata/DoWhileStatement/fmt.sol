pragma solidity ^0.8.8;

contract DoWhileStatement {
    function test() external {
        uint256 i;
        do {
            "test";
        } while (i != 0);

        do {} while (i != 0);

        bool someVeryVeryLongCondition;
        do {
            "test";
        } while (
            someVeryVeryLongCondition && !someVeryVeryLongCondition
                && !someVeryVeryLongCondition && someVeryVeryLongCondition
        );
    }
}
