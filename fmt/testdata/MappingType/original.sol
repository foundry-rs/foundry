contract X {
    type Y is bytes32;
}

contract Mapping {
    mapping(uint256 => X.Y) mapping1;
    mapping(uint256 key => uint256 value) mapping2;
    mapping(uint256 veryLongKeyName => uint256 veryLongValueName) mapping3;
    mapping(string anotherVeryLongKeyName => uint256 anotherVeryLongValueName) mapping4;
}