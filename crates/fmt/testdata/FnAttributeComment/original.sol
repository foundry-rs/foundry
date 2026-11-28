contract C {
    function someFunctionWithAnExtremelyLongNameThatForcesTheHeaderToBreakAcrossLines() /* @use-src 0:"input.sol", 1:"#utility.yul" */ public { }

    function shortOne() /* c */ public {}

    function mixedThenLine() /* a */ // b
    public {}

    function commentBeforeBody() internal returns (uint256 a) // note
    {
        a = 1;
    }

    function blockCommentBeforeBody() internal returns (uint256 a) /* note */ {
        a = 1;
    }
}

function freeFunctionBeforeBody() pure returns (uint256 a) // note
{
    a = 1;
}
