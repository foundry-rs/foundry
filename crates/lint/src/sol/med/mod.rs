use crate::sol::SolLint;

register_lints!(
    assert_state_change: (AssertStateChange, late, (ASSERT_STATE_CHANGE));
    dangerous_unary_operator: (DangerousUnaryOperator, early, (DANGEROUS_UNARY_OPERATOR));
    div_mul: (DivideBeforeMultiply, late, (DIVIDE_BEFORE_MULTIPLY));
    ecrecover: (Ecrecover, late, (ECRECOVER));
    incorrect_erc20_interface: (IncorrectERC20Interface, late, (INCORRECT_ERC20_INTERFACE));
    incorrect_erc721_interface: (IncorrectERC721Interface, late, (INCORRECT_ERC721_INTERFACE));
    incorrect_strict_equality: (IncorrectStrictEquality, late, (INCORRECT_STRICT_EQUALITY));
    tautology: (TypeBasedTautology, late, (TYPE_BASED_TAUTOLOGY));
    tx_origin: (TxOrigin, early, (TX_ORIGIN));
    uninitialized_local: (UninitializedLocal, late, (UNINITIALIZED_LOCAL));
    uninitialized_state_variables:
        (UninitializedStateVariables, late, (UNINITIALIZED_STATE_VARIABLES));
    unsafe_oz_erc721_mint: (UnsafeOzErc721Mint, late, (UNSAFE_OZ_ERC721_MINT));
    unsafe_typecast: (UnsafeTypecast, late, (UNSAFE_TYPECAST));
    unused_return: (UnusedReturn, late, (UNUSED_RETURN));
    locked_ether: (LockedEther, late, (LOCKED_ETHER));
    mapping_deletion: (MappingDeletion, late, (MAPPING_DELETION));
    non_reentrant_not_first: (NonReentrantNotFirst, late, (NON_REENTRANT_NOT_FIRST));
    weak_prng: (WeakPrng, early, (WEAK_PRNG));
    tautological_compare: (TautologicalCompare, late, (TAUTOLOGICAL_COMPARE));
);
