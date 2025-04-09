contract NamedFunctionCallExpression {
    struct SimpleStruct {
        uint256 val;
    }

    struct ComplexStruct {
        uint256 val;
        uint256 anotherVal;
        bool flag;
        uint256 timestamp;
    }

    struct
        StructWithAVeryLongNameThatExceedsMaximumLengthThatIsAllowedForFormatting {
            string whyNameSoLong;
        }

    function test() external {
        SimpleStruct memory simple = SimpleStruct({val: 0});

        ComplexStruct memory complex = ComplexStruct({
            val: 1,
            anotherVal: 2,
            flag: true,
            timestamp: block.timestamp
        });

        StructWithAVeryLongNameThatExceedsMaximumLengthThatIsAllowedForFormatting
            memory long =
            StructWithAVeryLongNameThatExceedsMaximumLengthThatIsAllowedForFormatting({
                whyNameSoLong: "dunno"
            });

        SimpleStruct memory simple2 = SimpleStruct({ // comment1
            /* comment2 */
            val: /* comment3 */ 0
        });

        SimpleStruct memory simple3 = SimpleStruct({
            /* comment4 */
            // comment5
            val: // comment6
                0 // comment7
                // comment8
        });
    }
}
