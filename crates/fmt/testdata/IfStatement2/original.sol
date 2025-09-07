contract IfStatement {

    function test() external {
        bool anotherLongCondition;

        if (condition && ((condition || anotherLongCondition)
        )
        ) execute();
    }
}