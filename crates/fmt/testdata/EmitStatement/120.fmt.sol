// config: line_length = 120
// config: prefer_compact = "none"
event NewEvent(address beneficiary, uint256 index, uint64 timestamp, uint64 endTimestamp);

function emitEvent() {
    emit NewEvent(beneficiary, _vestingBeneficiaries.length - 1, uint64(block.timestamp), endTimestamp);

    emit NewEvent( /* beneficiary */
        beneficiary,
        /* index */
        _vestingBeneficiaries.length - 1,
        /* timestamp */
        uint64(block.timestamp),
        /* end timestamp */
        endTimestamp
    );

    emit NewEvent(
        beneficiary, // beneficiary
        _vestingBeneficiaries.length - 1, // index
        uint64(block.timestamp), // timestamp
        endTimestamp // end timestamp
    );

    // https://github.com/foundry-rs/foundry/issues/12029
    emit OperatorSharesDecreased(
        defaultOperator,
        address(0),
        strategyMock,
        depositAmount / 6 // 1 withdrawal not queued so decreased
    );

    // https://github.com/foundry-rs/foundry/issues/12146
    emit ISablierComptroller.DisableCustomFeeUSD(
        protocol_protocol,
        caller_caller,
        user_users.sender,
        previousMinFeeUSD_0,
        newMinFeeUSD_feeUSD
    );
    emit ISablierComptroller.DisableCustomFeeUSD({
        protocol: protocol,
        caller: caller,
        user: users.sender,
        previousMinFeeUSD: 0,
        newMinFeeUSD: feeUSD
    });

    emit ISablierLockupLinear.CreateLockupLinearStream({
        streamId: streamId,
        commonParams: Lockup.CreateEventCommon({
            funder: msg.sender,
            sender: sender,
            recipient: recipient,
            depositAmount: depositAmount
        }),
        cliffTime: cliffTime,
        unlockAmounts: unlockAmounts
    });
}
