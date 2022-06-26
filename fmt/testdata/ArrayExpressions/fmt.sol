contract ArrayExpressions {
    function test() external {
        uint256[10] memory sample;

        uint256 length = 10;
        uint256[] memory sample2 = new uint[](length);

        uint256[] /* comment1 */ memory /* comment2 */ sample3; // comment3

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
            4:
                    // comment5
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
    }
}