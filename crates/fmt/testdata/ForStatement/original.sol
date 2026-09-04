pragma solidity ^0.8.8;

contract ForStatement {
    function test() external {
        for
    (uint256 i1
        ; i1 < 10;      i1++)
    {
             i1++;
            }

        uint256 i2;
        for(++i2;i2<10;i2++)

        {}

        uint256 veryLongVariableName = 1000;
        for ( uint256 i3; i3 < 10
        && veryLongVariableName>999 &&      veryLongVariableName< 1001
        ; i3++)
        { i3 ++ ; }

        for ( uint256 i3; i3 < 10
        && veryLongVariableName>900 &&      veryLongVariableName< 999
        ; i3++)
        { i3 ++ ; }


        for (type(uint256).min;;) {}

        for (;;) { "test" ; }

        for (uint256 i4; i4< 10; i4++) i4++;

        for (uint256 i5; ;)
            for (uint256 i6 = 10; i6 > i5; i6--)
                i5++;
        
        for (;;) doIt();
        for (;;) { doIt(); doIt(); }
        
        while (c) for (;;) x();
    }

    function bracedTrailingComment() external {
        uint256 x;
        for (uint256 i = 0; i < 10; ++i) { // step
            x++;
        }
    }

    function bracelessTrailingComment() external {
        uint256 x;
        for (uint256 i = 0; i < 10; ++i) x++; // step
    }

    function emptyBodyTrailingComment() external {
        for (uint256 i = 0; i < 10; ++i) { // step
        }
    }

    function leadingCommentUnaffected() external {
        // leading comment, must not move
        for (uint256 i = 0; i < 10; ++i) {
            i;
        }
    }

    function commentAfterHeaderNoCondNoNext() external {
        for (uint256 i = 0;;) // after header
        {}
    }

    function missingConditionTrailingComment() external {
        for (uint256 i = 0; ; ++i) { // c
            i;
        }
    }

    function missingIncrementTrailingComment() external {
        for (uint256 i = 0; i < 10;) { // c
            i;
        }
    }
}
