// config: line_length = 110
function repros() public {
    require(
        keccak256(abi.encodePacked("this is a long string"))
            == keccak256(abi.encodePacked("some other long string")),
        "string mismatch"
    );

    address lerp =
        LerpFactoryLike(lerpFab()).newLerp(_name, _target, _what, _startTime, _start, _end, _duration);

    (oracleRouter, eVault) = execute(
        oracleRouterFactory, deployRouterForOracle, eVaultFactory, upgradable, asset, oracle, unitOfAccount
    );

    if (eVault == address(0)) {
        eVault = address(
            GenericFactory(eVaultFactory)
                .createProxy(address(0), true, abi.encodePacked(asset, address(0), address(0)))
        );
    }

    content = string.concat(
        "{\"description\": \"",
        description,
        "\", \"name\": \"0x Settler feature ",
        ItoA.itoa(Feature.unwrap(feature)),
        "\"}\n"
    );

    oracleInfo = abi.encode(
        LidoOracleInfo({base: IOracle(oracleAddress).WSTETH(), quote: IOracle(oracleAddress).STETH()})
    );

    return someFunction().getValue().modifyValue().negate().scaleBySomeFactor(1000).transformToTuple();

    SnapshotRegistry(adapterRegistry)
        .add(adapter, LidoFundamentalOracle(adapter).WSTETH(), LidoFundamentalOracle(adapter).WETH());

    (bool success, bytes memory data) = GenericFactory(eVaultFactory).implementation()
        .staticcall(abi.encodePacked(EVCUtil.EVC.selector, uint256(0), uint256(0)));

    IEVC.BatchItem[] memory items = new IEVC.BatchItem[](3);

    items[0] = IEVC.BatchItem({
        onBehalfOfAccount: user,
        targetContract: address(eGRT),
        value: 0,
        data: abi.encodeCall(IERC4626.withdraw, (1500e18, address(swapper), user))
    });
    items[1] = IEVC.BatchItem({
        onBehalfOfAccount: user,
        targetContract: address(swapper),
        value: 0,
        data: abi.encodeCall(Swapper.multicall, multicallItems)
    });
    items[2] = IEVC.BatchItem({
        onBehalfOfAccount: user,
        targetContract: address(swapVerifier),
        value: 0,
        data: abi.encodeCall(
            swapVerifier.verifyDebtMax, (address(eSTETH), user, exactOutTolerance, type(uint256).max)
        )
    });

    uint256 fork = vm.createSelectFork("arbitrum", bytes32(0xdeadc0ffeedeadbeef));

    ConstructorVictim victim = new ConstructorVictim(sender, "msg.sender", "not set during prank");

    vm._expectCheatcodeRevert("short msg doesn't break");
    vm._expectCheatcodeRevert("failed parsing as `uint256`: missing hex prefix for hex string");
    vm.thisIsJustAReallyLongMemberWithoutAcall.LetsSeeHowItBreaks.willItBreakAsIntendedOrNot;

    bytes4[] memory targets = new bytes4[](0);
    targets[0] = FuzzArtifactSelector("TargetArtifactSelectors.t.sol:Hi", selectors);

    emit IERC712View.Transfer(Create3.predict(_salt, address(_deployer)), address(o), id);

    return _verifyDeploymentRootHash(_getMerkleRoot(proof, hash), originalOwner)
        .ternary(IERC1271.isValidSignature.selector, bytes4(0xffffffff));
}

function returnLongBinaryOp() returns (bytes32) {
    return bytes32(
        uint256(Feature.unwrap(feature)) << 128 | uint256(block.chainid) << 64 | uint256(Nonce.unwrap(nonce))
    );
}

contract Repros {
    function test() public {
        uint256 globalBuyAmount =
            Take.take(state, notes, uint32(IPoolManager.take.selector), recipient, minBuyAmount);
        uint256 globalBuyAmount =
            Take.take(state, notes, uint32(IPoolManager.take.selector), recipient, minBuyAmount);

        {
            u.executionData = _transferExecution(address(paymentToken), address(0xabcd), 1 ether);
            u.executionData = _transferExecution(address(paymentToken), address(0xabcd), 1 ether);
        }

        ISettlerBase.AllowedSlippage memory allowedSlippage = ISettlerBase.AllowedSlippage({
            recipient: payable(address(0)), buyToken: IERC20(address(0)), minAmountOut: 0
        });
        ISettlerBase.AllowedSlippage memory allowedSlippage = ISettlerBase.AllowedSlippage({
            recipient: payable(address(0)), buyToken: IERC20(address(0)), minAmountOut: 0
        });

        ISignatureTransfer.PermitTransferFrom memory permit = defaultERC20PermitTransfer(
            address(fromToken()),
            amount(),
            0 /* nonce */
        );
        ISignatureTransfer.PermitTransferFrom memory permit = defaultERC20PermitTransfer(
            address(fromToken()),
            amount(),
            0 /* nonce */
        );

        // https://github.com/foundry-rs/foundry/issues/11834
        CurrenciesOutOfOrderOrEqual.selector
            .revertWith(Currency.unwrap(key.currency0), Currency.unwrap(key.currency1));

        nestedStruct.withCalls.thatCause
            .aBreak(
                param1,
                param2,
                param3 // long line
            );

        // https://github.com/foundry-rs/foundry/issues/11835
        feeGrowthInside0X128 =
            self.feeGrowthGlobal0X128 - lower.feeGrowthOutside0X128 - upper.feeGrowthOutside0X128;
        feeGrowthInside0X128 =
            self.feeGrowthGlobal0X128 - lower.feeGrowthOutside0X128 - upper.feeGrowthOutside0X128;

        // https://github.com/foundry-rs/foundry/issues/11875
        lpTail = LpPosition({
            tickLower: posTickLower, tickUpper: posTickUpper, liquidity: lpTailLiquidity, id: uint16(id)
        });
    }

    // https://github.com/foundry-rs/foundry/issues/11834
    function test_ffi_fuzz_addLiquidity_defaultPool(IPoolManager.ModifyLiquidityParams memory paramSeed)
        public
    {
        a = 1;
    }

    // https://github.com/foundry-rs/foundry/issues/12324
    function test_longCallWithOpts() {
        flow.withdraw{value: FLOW_MIN_FEE_WEI}({
            streamId: defaultStreamId, to: users.eve, amount: WITHDRAW_AMOUNT_6D
        });
        flow.withdraw{
            value: FLOW_MIN_FEE_WEI /* cmnt */
        }({
            streamId: defaultStreamId,
            to: users.eve,
            /* cmnt */
            amount: WITHDRAW_AMOUNT_6D
        });
        flow.withdraw{value: FLOW_MIN_FEE_WEI}({ // cmnt
            streamId: defaultStreamId, to: users.eve, amount: WITHDRAW_AMOUNT_6D
        });
    }
}
