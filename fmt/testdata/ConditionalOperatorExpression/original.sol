contract TernaryExpression {
    function test() external {
        bool condition;
        bool someVeryVeryLongConditionUsedInTheTernaryExpression;

        condition ? 0 : 1;

        someVeryVeryLongConditionUsedInTheTernaryExpression ? 1234567890 : 987654321;

        condition /* comment1 */ ? /* comment2 */ 1001 /* comment3 */ : /* comment4 */ 2002;

        // comment5
        someVeryVeryLongConditionUsedInTheTernaryExpression ? 1
        // comment6
        :
        // comment7
        0; // comment8

        uint256 amount = msg.value > 0
            ? msg.value
            : parseAmount(IERC20(asset).balanceOf(msg.sender), msg.data);

        uint256 amount = msg.value > 0
            ? msg.value
            // comment9
            : parseAmount(IERC20(asset).balanceOf(msg.sender), msg.data);

        uint amount = msg.value > 0
            // comment10
            ? msg.value
            : parseAmount(IERC20(asset).balanceOf(msg.sender), msg.data);
    }
}