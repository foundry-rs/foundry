// comment 1
uint256 constant example1 = 1;

// comment 2
// comment 3
uint256 constant example2 = 2; // comment 4

uint256 constant example3 = /* comment 5 */ 3; // comment 6

// comment 7
contract SampleContract {
    // comment 8
    constructor() { /* comment 9 */ } // comment 10

    // comment 11
    function max( /* comment 13 */
        uint256 arg1,
        uint256 /* comment 14 */ arg2,
        uint256 /* comment 15 */
    )
        // comment 16
        external /* comment 17 */
        pure
        returns (uint256)
    // comment 18
    { // comment 19
        return arg1 > arg2 ? arg1 : arg2;
    }
}

// comment 20
contract /* comment 21 */ ExampleContract is /* comment 22 */ SampleContract {}

