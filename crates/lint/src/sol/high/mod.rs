use crate::sol::SolLint;

register_lints!(
    arbitrary_send_erc20:
        (ArbitrarySendErc20, late, (ARBITRARY_SEND_ERC20, ARBITRARY_SEND_ERC20_PERMIT));
    arbitrary_send_eth: (ArbitrarySendEth, late, (ARBITRARY_SEND_ETH));
    controlled_delegatecall: (ControlledDelegatecall, late, (CONTROLLED_DELEGATECALL));
    encode_packed_collision: (EncodedPackedCollision, late, (ENCODE_PACKED_COLLISION));
    enumerable_loop_removal: (EnumerableLoopRemoval, late, (ENUMERABLE_LOOP_REMOVAL));
    function_selector_collision: (FunctionSelectorCollision, late, (FUNCTION_SELECTOR_COLLISION));
    incorrect_exp: (IncorrectExp, late, (INCORRECT_EXP));
    incorrect_shift: (IncorrectShift, early, (INCORRECT_SHIFT));
    protected_vars: (ProtectedVars, late, (PROTECTED_VARS));
    reentrancy: (ReentrancyEth, late, (REENTRANCY_BALANCE, REENTRANCY_ETH, REENTRANCY_NO_ETH));
    unchecked_calls:
        (UncheckedCall, early, (UNCHECKED_CALL)),
        (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER));
    unprotected_initializer: (UnprotectedInitializer, late, (UNPROTECTED_INITIALIZER));
    rtlo: (Rtlo, early, (RTLO));
);
