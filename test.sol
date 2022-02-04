contract Contract {
    /**
     * @dev RAI's OracleRelayer does not have a view method to read what the current redemption
     * price is, so this method caches the most recent value as often as possible. It's called at
     * the start of every non-view method to update and store the current redemption price
     */
    function updateRedemptionPrice() public returns (uint256) {
        // These are trusted external calls, so it's ok that we call them before modifying state in other methods
        lastRedemptionPrice = oracleRelayer.redemptionPrice(); // non-payable (i.e. non-view) method
        lastRedemptionRate = oracleRelayer.redemptionRate(); // view method

        // We just updated the OracleRelayer's redemption price which sets `oracleRelayer.redemptionPriceUpdateTime()`
        // to the current time. Therefore we can set our cached value directly to the current time, which avoids
        // saves gas by avoiding another external call to read `redemptionPriceUpdateTime`
        lastRedemptionPriceUpdateTime = now;
        return lastRedemptionPrice;
    }
}
