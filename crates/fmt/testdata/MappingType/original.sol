contract X {
    type Y is bytes32;
}

type SomeVeryLongTypeName is uint256;

contract Mapping {
    mapping(uint256 => X.Y) mapping1;
    mapping(uint256 key => uint256 value) mapping2;
    mapping(uint256 veryLongKeyName => uint256 veryLongValueName) mapping3;
    mapping(string anotherVeryLongKeyName => uint256 anotherVeryLongValueName) mapping4;
    mapping(SomeVeryLongTypeName anotherVeryLongKeyName => uint256 anotherVeryLongValueName) mapping5;

    mapping(
            
            // comment1
    uint256 key => uint256 value
// comment2
    ) mapping6;
    mapping( /* comment3 */
        uint256 /* comment4 */ key /* comment5 */ => /* comment6 */ uint256 /* comment7 */ value /* comment8 */ /* comment9 */
    )  /* comment10 */ mapping7;
}