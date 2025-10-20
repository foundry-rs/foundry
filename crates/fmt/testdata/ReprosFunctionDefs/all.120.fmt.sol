// config: line_length = 120
// config: multiline_func_header = "all"
contract Repros {
    // https://github.com/foundry-rs/foundry/issues/12109
    function calculateStreamedPercentage(
        uint128 streamedAmount,
        uint128 depositedAmount
    )
        internal
        pure
        returns (uint256)
    {
        a = 1;
    }
}
