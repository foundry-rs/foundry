use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod assert_state_change;
use assert_state_change::ASSERT_STATE_CHANGE;

mod div_mul;
use div_mul::DIVIDE_BEFORE_MULTIPLY;

mod incorrect_erc20_interface;
use incorrect_erc20_interface::INCORRECT_ERC20_INTERFACE;

mod incorrect_erc721_interface;
use incorrect_erc721_interface::INCORRECT_ERC721_INTERFACE;

mod incorrect_strict_equality;
use incorrect_strict_equality::INCORRECT_STRICT_EQUALITY;

mod tautology;
use tautology::TYPE_BASED_TAUTOLOGY;

mod tx_origin;
use tx_origin::TX_ORIGIN;

mod uninitialized_local;
use uninitialized_local::UNINITIALIZED_LOCAL;

mod uninitialized_state_variables;
use uninitialized_state_variables::UNINITIALIZED_STATE_VARIABLES;

mod unsafe_typecast;
use unsafe_typecast::UNSAFE_TYPECAST;

mod unused_return;
use unused_return::UNUSED_RETURN;

mod locked_ether;
use locked_ether::LOCKED_ETHER;

mod non_reentrant_not_first;
use non_reentrant_not_first::NON_REENTRANT_NOT_FIRST;

mod tautological_compare;
use tautological_compare::TAUTOLOGICAL_COMPARE;

mod weak_prng;
use weak_prng::WEAK_PRNG;

register_lints!(
    (AssertStateChange, late, (ASSERT_STATE_CHANGE)),
    (DivideBeforeMultiply, late, (DIVIDE_BEFORE_MULTIPLY)),
    (IncorrectERC20Interface, late, (INCORRECT_ERC20_INTERFACE)),
    (IncorrectERC721Interface, late, (INCORRECT_ERC721_INTERFACE)),
    (IncorrectStrictEquality, late, (INCORRECT_STRICT_EQUALITY)),
    (TypeBasedTautology, late, (TYPE_BASED_TAUTOLOGY)),
    (TxOrigin, early, (TX_ORIGIN)),
    (UninitializedLocal, late, (UNINITIALIZED_LOCAL)),
    (UninitializedStateVariables, late, (UNINITIALIZED_STATE_VARIABLES)),
    (UnsafeTypecast, late, (UNSAFE_TYPECAST)),
    (UnusedReturn, late, (UNUSED_RETURN)),
    (LockedEther, late, (LOCKED_ETHER)),
    (NonReentrantNotFirst, late, (NON_REENTRANT_NOT_FIRST)),
    (WeakPrng, early, (WEAK_PRNG)),
    (TautologicalCompare, late, (TAUTOLOGICAL_COMPARE))
);
