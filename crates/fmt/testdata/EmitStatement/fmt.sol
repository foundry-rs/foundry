// config: line_length = 80
event NewEvent(
    address beneficiary, uint256 index, uint64 timestamp, uint64 endTimestamp
);

function emitEvent() {
    emit NewEvent(
        beneficiary,
        _vestingBeneficiaries.length - 1,
        uint64(block.timestamp),
        endTimestamp
    );

    emit NewEvent(
        /* beneficiary */
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
}
