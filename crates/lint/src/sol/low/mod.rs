use crate::sol::SolLint;

mod payable_loop;

register_lints!(
    block_timestamp: (BlockTimestamp, late, (BLOCK_TIMESTAMP));
    calls_loop: (CallsLoop, late, (CALLS_LOOP));
    delegatecall_loop: (DelegatecallLoop, late, (DELEGATECALL_LOOP));
    deprecated_oz_function: (DeprecatedOzFunction, late, (DEPRECATED_OZ_FUNCTION));
    empty_block: (EmptyBlock, early, (EMPTY_BLOCK));
    incorrect_modifier: (IncorrectModifier, late, (INCORRECT_MODIFIER));
    inconsistent_type_names: (InconsistentTypeNames, project, (INCONSISTENT_TYPE_NAMES));
    msg_value_loop: (MsgValueLoop, late, (MSG_VALUE_LOOP));
    missing_events_access_control:
        (MissingEventsAccessControl, late, (MISSING_EVENTS_ACCESS_CONTROL));
    missing_events_arithmetic: (MissingEventsArithmetic, late, (MISSING_EVENTS_ARITHMETIC));
    missing_zero_check: (MissingZeroCheck, late, (MISSING_ZERO_CHECK));
    return_bomb: (ReturnBomb, late, (RETURN_BOMB));
    reentrancy_events: (ReentrancyEvents, late, (REENTRANCY_EVENTS));
    require_revert_in_loop: (RequireRevertInLoop, late, (REQUIRE_REVERT_IN_LOOP));
    solmate_safe_transfer_lib: (SolmateSafeTransferLib, late, (SOLMATE_SAFE_TRANSFER_LIB));
);
