contract SimpleComments {
    mapping(address /* asset */ => address /* router */) public router;

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

    function test4() public view {
        uint256 abc; // long postfix comment that exceeds line width. the comment should be split and carried over to the next line
        uint256 abc2; // reallylongsinglewordcommentthatexceedslinewidththecommentshouldbesplitandcarriedovertothenextline

        // long prefix comment that exceeds line width. the comment should be split and carried over to the next line
        // reallylongsinglewordcommentthatexceedslinewidththecommentshouldbesplitandcarriedovertothenextline
        uint256 c;

        /* a really really long multiline prefix comment that exceeds line width */
        uint256 d; /* a really really long multiline postfix comment that exceeds line width */
    }
}

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
