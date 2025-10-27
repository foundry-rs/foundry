contract IfStatement {
    function test() external {
        bool anotherLongCondition;

        if (condition && ((condition || anotherLongCondition))) execute();
    }

    // https://github.com/foundry-rs/foundry/issues/12102
    function repro() external {
        for (uint256 i; i < len; ++i) {
            proportions[i] = totalDepositedTvl == 0
                ? 0
                : Math.mulDiv(
                    vaultUsdValue[i],
                    1e18,
                    totalDepositedTvl,
                    Math.Rounding.Floor
                );
            proportions[i] = totalDepositedTvl == 0
                ? 0
                : Math.mulDiv(
                    vaultUsdValue[i],
                    1e18,
                    totalDepositedTvl,
                    Math.Rounding.Floor
                );
        }
    }
}
