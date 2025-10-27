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
    sum += getPrice(
        ACCELERATE_STARTING_PRICE,
        ACCELERATE_PER_PERIOD_DECREASE,
        idleTicks,
        actionsSold[ActionType.ACCELERATE] + i,
        ACCELERATE_SELL_PER_TICK
    ) / 1e18;
    other += 1e18
        / getPrice(
            ACCELERATE_STARTING_PRICE,
            ACCELERATE_PER_PERIOD_DECREASE,
            idleTicks,
            actionsSold[ActionType.ACCELERATE] + i,
            ACCELERATE_SELL_PER_TICK
        );
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

function test_nested() {
    require(
        keccak256(abi.encodePacked("some long string"))
            == keccak256(abi.encodePacked("some other long string")),
        "string mismatch"
    );

    state.zeroForOne = IERC20(Currency.unwrap(state.poolKey1.currency0))
        == IERC20(Currency.unwrap(state.poolKey0.curerncy1));

    coreAddresses.evc == address(0)
        && coreAddresses.protocolConfig == address(0)
        && coreAddresses.sequenceRegistry == address(0)
        && coreAddresses.balanceTracker == address(0)
        && coreAddresses.permit2 == address(0);

    return spender == ownerOf(tokenId) || getApproved[tokenId] == spender
        || isApprovedForAll[ownerOf(tokenId)][spender];
}

function new_y(
    uint256 x,
    uint256 dx,
    uint256 x_basis,
    uint256 y,
    uint256 y_basis
) external pure returns (uint256) {
    return _get_y(
        x * _VELODROME_TOKEN_BASIS / x_basis,
        dx * _VELODROME_TOKEN_BASIS / x_basis,
        y * _VELODROME_TOKEN_BASIS / y_basis
    ) * y_basis / _VELODROME_TOKEN_BASIS
        * aReallyLongIdentifierThatMakesTheOperatorExpressionBreak;
}

contract Repro {
    bytes4 public constant MINIMAL_INTERFACE_ID =
        this.calculateMinFeeWeiFor.selector ^ this.convertUSDFeeToWei.selector
            ^ this.execute.selector ^ this.getMinFeeUSDFor.selector;
    bool isTestnet = chainId == ARBITRUM_SEPOLIA || chainId == BASE_SEPOLIA
        || chainId == MODE_SEPOLIA || chainId == OPTIMISM_SEPOLIA
        || chainId == SEPOLIA;

    function test() {
        assign = this.calculateMinFeeWeiFor.selector
            ^ this.convertUSDFeeToWei.selector ^ this.execute.selector
            ^ this.getMinFeeUSDFor.selector;
        isMainnet = chainId == ABSTRACT || chainId == ARBITRUM
            || chainId == AVALANCHE || chainId == BASE || chainId == BERACHAIN
            || chainId == BLAST || chainId == BSC || chainId == CHILIZ
            || chainId == COREDAO || chainId == ETHEREUM || chainId == GNOSIS
            || chainId == HYPEREVM || chainId == LIGHTLINK || chainId == LINEA
            || chainId == MODE || chainId == MORPH || chainId == OPTIMISM
            || chainId == POLYGON || chainId == SCROLL || chainId == SEI
            || chainId == SOPHON || chainId == SUPERSEED || chainId == SONIC
            || chainId == UNICHAIN || chainId == XDC || chainId == ZKSYNC;

        callsGas += (3 * FixedPointMathLib.divUp(paramsLength, 32))
            + FixedPointMathLib.mulDivUp(paramsLength, paramsLength, 524_288);
    }
}
