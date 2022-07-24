// config: int_types = "short"
contract Contract {
    uint constant UINT256_IMPL = 0;
    uint8 constant UINT8 = 1;
    uint128 constant UINT128 = 2;
    uint constant UINT256_EXPL = 3;

    int constant INT256_IMPL = 4;
    int8 constant INT8 = 5;
    int128 constant INT128 = 6;
    int constant INT256_EXPL = 7;

    function test(
        uint uint256_impl,
        uint8 uint8_var,
        uint128 uint128_var,
        uint uint256_expl,
        int int256_impl,
        int8 int8_var,
        int128 int128_var,
        int int256_expl
    )
        public
    {
        // do something
    }
}
