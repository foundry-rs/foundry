// config: line_length = 40
// config: override_spacing = true
contract Contract layout at 69 {
    bytes32 transient a;

    bytes32 private constant BYTES = 0;
    bytes32
        private
        constant
        override (Base1) BYTES = 0;
    bytes32
        private
        constant
        override (Base1, Base2) BYTES = 0;
    bytes32
        private
        constant
        override BYTES = 0;
    bytes32
        private
        constant
        override BYTES_VERY_VERY_VERY_LONG = 0;
    bytes32
        private
        constant
        override (
            Base1,
            Base2,
            SomeLongBaseContract,
            AndAnotherVeryLongBaseContract,
            Imported.Contract
        ) BYTES_OVERRIDDEN = 0;

    bytes32 private constant BYTES =
        0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32
        private
        constant
        override BYTES =
            0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32
        private
        constant
        override BYTES_VERY_VERY_VERY_LONG =
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
