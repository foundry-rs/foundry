contract Contract {
    bytes32 constant private BYTES;
    bytes32 private constant override immutable BYTES;
    bytes32 private constant override immutable BYTES_VERY_VERY_VERY_LONG;

    bytes32 constant private BYTES =
        0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32 private constant override immutable BYTES =
        0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32 private constant override immutable BYTES_VERY_VERY_VERY_LONG =
        0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;
    bytes32 private constant BYTES_VERY_VERY_LONG =
        0x035aff83d86937d35b32e04f0ddc6ff469290eef2f1b692d8a815c89404d4749;

    uint constant POWER_EXPRESSION = 10 ** 27;
    uint constant ADD_EXPRESSION = 1 + 2;
}
