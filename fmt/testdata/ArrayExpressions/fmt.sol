contract ArrayExpressions {
    function test() external {
        /* ARRAY SUBSCRIPT */
        uint256[10] memory sample;

        uint256 length = 10;
        uint256[] memory sample2 = new uint[](length);

        uint256[] /* comment1 */ memory /* comment2 */ sample3; // comment3

        /* ARRAY SLICE */
        msg.data[4:];
        msg.data[:msg.data.length];
        msg.data[4:msg.data.length];

        msg.data[
            // comment1
            4:
        ];
        msg.data[
            : /* comment2 */ msg.data.length // comment3
        ];
        msg.data[
            // comment4
            4: // comment5
                msg.data.length /* comment6 */
        ];

        uint256
            someVeryVeryVeryLongVariableNameThatDenotesTheStartOfTheMessageDataSlice =
                4;
        uint256
            someVeryVeryVeryLongVariableNameThatDenotesTheEndOfTheMessageDataSlice =
                msg.data.length;
        msg.data[
            someVeryVeryVeryLongVariableNameThatDenotesTheStartOfTheMessageDataSlice:
        ];
        msg.data[
            :someVeryVeryVeryLongVariableNameThatDenotesTheEndOfTheMessageDataSlice
        ];
        msg.data[
            someVeryVeryVeryLongVariableNameThatDenotesTheStartOfTheMessageDataSlice:
                someVeryVeryVeryLongVariableNameThatDenotesTheEndOfTheMessageDataSlice
        ];

        /* ARRAY LITERAL */
        [1, 2, 3];

        uint256 someVeryVeryLongVariableName = 0;
        [
            someVeryVeryLongVariableName,
            someVeryVeryLongVariableName,
            someVeryVeryLongVariableName
        ];
        uint256[3] memory literal = [
            someVeryVeryLongVariableName,
            someVeryVeryLongVariableName,
            someVeryVeryLongVariableName
        ];

        uint8[3] memory literal2 = /* comment7 */ [ // comment8
            1,
            2, /* comment9 */
            3 // comment10
        ];
        uint256[1] memory literal3 =
            [ /* comment11 */ someVeryVeryLongVariableName /* comment13 */ ];
    }
}
