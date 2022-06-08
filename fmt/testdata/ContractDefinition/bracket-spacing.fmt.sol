// config: line-length=160
// config: bracket-spacing=true
contract ContractDefinition is Contract1, Contract2, Contract3, Contract4, Contract5 { }

// comment 7
contract SampleContract {
    // comment 8
    constructor() { /* comment 9 */ } // comment 10

    // comment 11
    function max( /* comment 13 */ uint256 arg1, uint256 /* comment 14 */ arg2, uint256 /* comment 15 */ )
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
contract /* comment 21 */ ExampleContract is /* comment 22 */ SampleContract { }

