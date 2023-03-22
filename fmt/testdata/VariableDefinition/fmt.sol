// config: line_length = 40
contract Contract {
    bytes32 private constant BYTES;
    bytes32
        private
        constant
        override(Base1) BYTES;
    bytes32
        private
        constant
        override(Base1, Base2) BYTES;
    bytes32
        private
        constant
        immutable
        override BYTES;
    bytes32
        private
        constant
        immutable
        override
        BYTES_VERY_VERY_VERY_LONG;
    bytes32
        private
        constant
        override(
            Base1,
            Base2,
            SomeLongBaseContract,
            AndAnotherVeryLongBaseContract,
            Imported.Contract
        ) BYTES_OVERRIDDEN;

    bytes32 private constant BYTES =
        0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32
        private
        constant
        immutable
        override BYTES =
            0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32
        private
        constant
        immutable
        override
        BYTES_VERY_VERY_VERY_LONG =
            0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32 private constant
        BYTES_VERY_VERY_LONG =
            0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;

    uint256 constant POWER_EXPRESSION =
        10 ** 27;
    uint256 constant ADDED_EXPRESSION =
        1 + 2;

    // comment 1
    uint256 constant example1 = 1;
    // comment 2
    // comment 3
    uint256 constant example2 = 2; // comment 4
    uint256 constant example3 = /* comment 5 */
        3; // comment 6
}
