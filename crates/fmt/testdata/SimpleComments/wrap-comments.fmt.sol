// config: line_length = 60
// config: wrap_comments = true
contract SimpleComments {
    uint40 constant PERIOD = uint40(12345); // ~578 days
    // Represents the depletion timestamp
    uint40 constant WARP_PERIOD = FEB_1_2025 + PERIOD;

    //´:°•.°+.*•´.*:˚.°*.˚•´.°:°•.°•.*•´.*:˚.°*.˚•´.°:°•.°+.*•´.*:
    // VARIABLES
    //.•°:°.´+˚.*°.˚:*.´•*.+°.•°:´*.´•*.•°.•°:°.´:•˚°.*°.˚:*.´+°.•

    mapping(address /* asset */ => address /* router */)
        public router;

    /*´:°•.°+.*•´.*:˚.°*.˚•´.°:°•.°•.*•´.*:˚.°*.˚•´.°:°•.°+.*•´.*:*/
    /*                         FUNCTIONS
    */
    /*.•°:°.´+˚.*°.˚:*.´•*.+°.•°:´*.´•*.•°.•°:°.´:•˚°.*°.˚:*.´+°.•*/

    constructor() {
        // TODO: do this and that

        uint256 a = 1;

        // TODO: do that and this
        // or maybe
        // smth else
    }

    function test() public view {
        // do smth here

        // then here

        // cleanup
    }

    function test2() public pure {
        uint256 a = 1;
        // comment 1
        // comment 2
        uint256 b = 2;
    }

    function test3() public view {
        uint256 a = 1; // comment

        // line comment
    }

    function test4() public view returns (uint256) {
        uint256 abc; // long postfix comment that exceeds
        // line width. the comment should be split and
        // carried over to the next line
        uint256 abc2; // reallylongsinglewordcommentthatexceedslinewidththecommentshouldbesplitandcarriedovertothenextline

        // long prefix comment that exceeds line width. the
        // comment should be split and carried over to the
        // next line
        // reallylongsinglewordcommentthatexceedslinewidththecommentshouldbesplitandcarriedovertothenextline
        uint256 c;

        /* a really really long prefix block comment that
        exceeds line width */
        uint256 d; /* a really really long postfix block
            comment that exceeds line width */

        uint256 value;
        return /* a long block comment that exceeds line
            width */ value;
        return /* a block comment that exceeds line width */
            value;
        return // a line comment that exceeds line width
            value;
    }

    // https://github.com/foundry-rs/foundry/issues/11836
    function test5() public {
        (
            /* poolIndex */,
            uint256 sellAmount1,
            uint256 buyAmount1,
            /* poolKey1 */,
            /* sellToken */,
            /* buyToken */,
            /* sellTokenBalanceBefore */,
            uint256 buyTokenBalanceBefore1,
            /* hashMul */,
            /* hashMod */
        ) = _swapPre(
            2, TOTAL_SUPPLY / 1_000, false, zeroForOne1
        );
    }

    // https://github.com/foundry-rs/foundry/issues/12045
    function test6() {
        (
            // uint80 roundID
            ,
            int256 dataFeedAnswer,
            // uint startedAt
            ,
            uint256 updatedAt,
            // uint80 answeredInRound
        ) = dataFeedContract.latestRoundData();
    }
}

/*

██████╗ ██████╗ ██████╗ ████████╗███████╗███████╗████████╗
██╔══██╗██╔══██╗██╔══██╗╚══██╔══╝██╔════╝██╔════╝╚══██╔══╝
██████╔╝██████╔╝██████╔╝   ██║   █████╗  ███████╗   ██║
██╔═══╝ ██╔══██╗██╔══██╗   ██║   ██╔══╝  ╚════██║   ██║
██║     ██║  ██║██████╔╝   ██║   ███████╗███████║   ██║
╚═╝     ╚═╝  ╚═╝╚═════╝    ╚═╝   ╚══════╝╚══════╝   ╚═╝
*/
function asciiArt() {}

/*
 * @notice Here is my comment
 *       - item 1
 *       - item 2
 * Some equations:
 *     y = mx + b
 */
function test() {}
// comment after function

// comment with extra newlines

// some comment
// another comment

// eof comment
