contract TestConditionFormatting {
    function testConditions() public {
        uint256 newNumber = 5;

        if (newNumber % 2 == 0 || newNumber % 2 == 1 || newNumber != 0 || newNumber != 1 || newNumber != 2) {
            // do something
        }
    }
}
