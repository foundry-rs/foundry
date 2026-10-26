contract C {
    function someFunctionWithAnExtremelyLongNameThatForcesTheHeaderToBreakAcrossLines()
        /* @use-src 0:"input.sol", 1:"#utility.yul" */
        public {}

    function shortOne() /* c */ public {}

    function mixedThenLine()
        /* a */
        // b
        public {}
}
