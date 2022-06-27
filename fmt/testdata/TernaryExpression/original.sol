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
    }
}