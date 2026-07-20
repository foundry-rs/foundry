// https://github.com/foundry-rs/foundry/issues/15755
interface IVault {
    function updatePosition(uint256 positionId_, uint256 batchId_, address account_, address operator_, uint256 amount_, uint256 deadline_, bytes calldata data_) external;
}

contract NamedCallArgsInChain {
    function regular(address vault, uint256 positionId, uint256 batchId, address account, address operator, uint256 amount, uint256 deadline, bytes calldata data) external {
        IVault(vault).updatePosition({positionId_: positionId, batchId_: batchId, account_: account, operator_: operator, amount_: amount, deadline_: deadline, data_: data});
    }

    function overlongCallee(address vault, uint256 positionId, uint256 batchId) external {
        IExtremelyLongVaultInterfaceNameThatStillFits(vault).updateAnExtremelyLongPositionNameThatMakesTheCombinedCalleeOverflow({positionId_: positionId, batchId_: batchId, account_: address(0), operator_: address(0), amount_: 0, deadline_: 0, data_: ""});
    }

    function calleeAtLineBoundary(address vault, uint256 positionId, uint256 batchId) external {
        IVault(vault).updatePositionAtTheExactConfiguredLineLengthBoundaryWithoutForcingTheBaseInterfaceConversionToWrap({positionId_: positionId, batchId_: batchId});
    }

    function reviewCases(uint256 firstExtremelyLongValueName, uint256 secondExtremelyLongValueName) external {
        factory().foo(bar(firstExtremelyLongValueName)).baz({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
        factory() // preserve
            .item().update({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
        factory().item(/* preserve */ firstExtremelyLongValueName).update({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
        factory().items()[0].update({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
        (factory().item()).update({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
        foo{value: 1}().bar({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
        (foo{value: 1})().bar({firstExtremelyLongArgumentName: firstExtremelyLongValueName, secondExtremelyLongArgumentName: secondExtremelyLongValueName});
    }

    // https://github.com/foundry-rs/foundry/issues/15823
    function issue15823(uint256 redactedId, uint256 setId, bytes calldata redactedEngineData) external {
        RedactedRootConfiguration.getRedactedEngine({
            redactedId: redactedId
        }).initializeSetRedacted({
            setId_: setId, redactedId_: redactedId, redactedEngineData_: redactedEngineData
        });
    }

    function attempted(address vault, uint256 positionId, uint256 batchId, address account, address operator, uint256 amount, uint256 deadline, bytes calldata data) external {
        try IVault(vault).updatePosition({positionId_: positionId, batchId_: batchId, account_: account, operator_: operator, amount_: amount, deadline_: deadline, data_: data}) {} catch {}
    }
}
