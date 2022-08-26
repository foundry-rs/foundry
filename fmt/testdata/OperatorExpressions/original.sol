function test() {
    uint256 expr001 = (1 + 2) + 3;
    uint256 expr002 = 1 + (2 + 3);
    uint256 expr003 = 1 * 2 + 3;
    uint256 expr004 = (1 * 2) + 3;
    uint256 expr005 = 1 * (2 + 3);
    uint256 expr006 = 1 + 2 * 3;
    uint256 expr007 = (1 + 2) * 3;
    uint256 expr008 = 1 + (2 * 3);
    uint256 expr009 = 1 ** 2 ** 3;
    uint256 expr010 = 1 ** (2 ** 3);
    uint256 expr011 = (1 ** 2) ** 3;
    uint256 expr012 = ++expr011 + 1;
    bool expr013 = ++expr012 == expr011 - 1;
    bool expr014 = ++(++expr013)--;
    if (++batch.movesPerformed == drivers.length) createNewBatch();
    sum += getPrice(ACCELERATE_STARTING_PRICE, ACCELERATE_PER_PERIOD_DECREASE, idleTicks, actionsSold[ActionType.ACCELERATE] + i, ACCELERATE_SELL_PER_TICK) / 1e18;
    other += 1e18 / getPrice(ACCELERATE_STARTING_PRICE, ACCELERATE_PER_PERIOD_DECREASE, idleTicks, actionsSold[ActionType.ACCELERATE] + i, ACCELERATE_SELL_PER_TICK);
        if (
        op == 0x54 // SLOAD
        || op == 0x55 // SSTORE
        || op == 0xF0 // CREATE
        || op == 0xF1 // CALL
        || op == 0xF2 // CALLCODE
        || op == 0xF4 // DELEGATECALL
        || op == 0xF5 // CREATE2
        || op == 0xFA // STATICCALL
        || op == 0xFF // SELFDESTRUCT
    ) return false;
}
