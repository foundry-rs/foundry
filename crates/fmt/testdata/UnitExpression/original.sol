contract UnitExpression {
    function test() external {
        uint256 timestamp;
        timestamp = 1 seconds;
        timestamp = 1 minutes;
        timestamp = 1 hours;
        timestamp = 1 days;
        timestamp = 1 weeks;

        uint256 value;
        value = 1 wei;
        value = 1 gwei;
        value = 1 ether;

        uint256 someVeryVeryVeryLongVariableNameForTheMultiplierForEtherValue;

        value =  someVeryVeryVeryLongVariableNameForTheMultiplierForEtherValue * 1 /* comment1 */ ether; // comment2

        value = 1 // comment3
        // comment4
        ether; // comment5
    }
}