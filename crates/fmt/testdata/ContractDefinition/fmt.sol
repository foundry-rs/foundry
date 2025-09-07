contract ContractDefinition is
    Contract1,
    Contract2,
    Contract3,
    Contract4,
    Contract5
{}

// comment 7
contract SampleContract {
    // spaced comment 1

    // spaced comment 2
    // that spans multiple lines

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
    {
        // comment 19
        return arg1 > arg2 ? arg1 : arg2;
    }
}

// comment 20
contract /* comment 21 */ ExampleContract is /* comment 22 */ SampleContract {}

contract ERC20DecimalsMock is ERC20 {
    uint8 private immutable _decimals;

    constructor(string memory name_, string memory symbol_, uint8 decimals_)
        ERC20(name_, symbol_)
    {
        _decimals = decimals_;
    }
}
