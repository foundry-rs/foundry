// config: line_length = 120
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
}
