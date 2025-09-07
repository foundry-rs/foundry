contract RevertNamedArgsStatement {
    error EmptyError();
    error SimpleError(uint256 val);
    error ComplexError(uint256 val, uint256 ts, string message);
    error SomeVeryVeryVeryLongErrorNameWithNamedArgumentsThatExceedsMaximumLength(
        uint256 val, uint256 ts, string message
    );

    function test() external {
        revert ({ });

        revert EmptyError({});

        revert SimpleError({ val: 0 });

        revert ComplexError(
            {
                val: 0,
                    ts: block.timestamp,
                        message: "some reason"
            });
        
        revert SomeVeryVeryVeryLongErrorNameWithNamedArgumentsThatExceedsMaximumLength({ val: 0, ts: 0x00, message: "something unpredictable happened that caused execution to revert"});

        revert // comment1 
        ({});

         revert /* comment2 */ SimpleError /* comment3 */ ({ // comment4 
        val:0 // comment 5
        });
    }
}
