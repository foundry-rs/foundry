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
        uint a = 1;
        // comment 1
          // comment 2
        uint b = 2;
    }

    function test3() public view {
        uint256 a = 1; // comment

        // line comment
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