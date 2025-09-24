// config: line_length = 80
function test() public {
    require(
        keccak256(abi.encodePacked("this is a long string"))
            == keccak256(abi.encodePacked("some other long string")),
        "string mismatch"
    );

    address lerp = LerpFactoryLike(lerpFab())
        .newLerp(_name, _target, _what, _startTime, _start, _end, _duration);

    (oracleRouter, eVault) = execute(
        oracleRouterFactory,
        deployRouterForOracle,
        eVaultFactory,
        upgradable,
        asset,
        oracle,
        unitOfAccount
    );

    if (eVault == address(0)) {
        eVault = address(
            GenericFactory(eVaultFactory)
                .createProxy(
                    address(0),
                    true,
                    abi.encodePacked(asset, address(0), address(0))
                )
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
        LidoOracleInfo({
            base: IOracle(oracleAddress).WSTETH(),
            quote: IOracle(oracleAddress).STETH()
        })
    );

    return someFunction().getValue().modifyValue().negate()
        .scaleBySomeFactor(1000).transformToTuple();

    SnapshotRegistry(adapterRegistry)
        .add(
            adapter,
            LidoFundamentalOracle(adapter).WSTETH(),
            LidoFundamentalOracle(adapter).WETH()
        );

    (bool success, bytes memory data) = GenericFactory(eVaultFactory)
        .implementation()
        .staticcall(
            abi.encodePacked(EVCUtil.EVC.selector, uint256(0), uint256(0))
        );

    IEVC.BatchItem[] memory items = new IEVC.BatchItem[](3);

    items[0] = IEVC.BatchItem({
        onBehalfOfAccount: user,
        targetContract: address(eGRT),
        value: 0,
        data: abi.encodeCall(
            IERC4626.withdraw, (1500e18, address(swapper), user)
        )
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
            swapVerifier.verifyDebtMax,
            (address(eSTETH), user, exactOutTolerance, type(uint256).max)
        )
    });

    uint256 fork =
        vm.createSelectFork("arbitrum", bytes32(0xdeadc0ffeedeadbeef));

    ConstructorVictim victim =
        new ConstructorVictim(sender, "msg.sender", "not set during prank");

    vm._expectCheatcodeRevert("short msg doesn't break");
    vm._expectCheatcodeRevert(
        "failed parsing as `uint256`: missing hex prefix for hex string"
    );
    vm.thisIsJustAReallyLongMemberWithoutAcall.LetsSeeHowItBreaks
        .willItBreakAsIntendedOrNot;

    bytes4[] memory targets = new bytes4[](0);
    targets[0] =
        FuzzArtifactSelector("TargetArtifactSelectors.t.sol:Hi", selectors);

    emit IERC712View.Transfer(
        Create3.predict(_salt, address(_deployer)), address(o), id
    );

    return _verifyDeploymentRootHash(_getMerkleRoot(proof, hash), originalOwner)
        .ternary(IERC1271.isValidSignature.selector, bytes4(0xffffffff));
}

function returnLongBinaryOp() returns (bytes32) {
    return bytes32(
        uint256(Feature.unwrap(feature)) << 128 | uint256(block.chainid) << 64
            | uint256(Nonce.unwrap(nonce))
    );
}
