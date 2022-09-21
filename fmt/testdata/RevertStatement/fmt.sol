contract RevertStatement {
    error TestError(uint256, bool, string);

    function someVeryLongFunctionNameToGetDynamicErrorMessageString()
        public
        returns (string memory)
    {
        return "";
    }

    function test(string memory message) external {
        revert();

        revert( /* comment1 */ );

        revert();

        // comment2
        revert(
            // comment3
        );

        revert(message);

        revert(
            // comment4
            message // comment5 /* comment6 */
        );

        revert( /* comment7 */ /* comment8 */ message /* comment9 */ ); /* comment10 */ // comment11

        revert(
            string.concat(
                message,
                someVeryLongFunctionNameToGetDynamicErrorMessageString(
                    /* comment12 */
                )
            )
        );

        revert TestError(0, false, message);
        revert TestError(
            0, false, someVeryLongFunctionNameToGetDynamicErrorMessageString()
        );

        revert /* comment13 */ /* comment14 */ TestError( /* comment15 */
            1234567890, false, message
        );

        revert TestError( /* comment16 */
            1,
            true,
            someVeryLongFunctionNameToGetDynamicErrorMessageString() /* comment17 */
        );
    }
}
