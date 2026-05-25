use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod incorrect_shift;
mod reentrancy;
mod rtlo;
mod unchecked_calls;
mod unprotected_initializer;

use incorrect_shift::INCORRECT_SHIFT;
use reentrancy::REENTRANCY_UNLIMITED_GAS;
use rtlo::RTLO;
use unchecked_calls::{ERC20_UNCHECKED_TRANSFER, UNCHECKED_CALL};
use unprotected_initializer::UNPROTECTED_INITIALIZER;

register_lints!(
    (IncorrectShift, early, (INCORRECT_SHIFT)),
    (ReentrancyUnlimitedGas, late, (REENTRANCY_UNLIMITED_GAS)),
    (UncheckedCall, early, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER)),
    (UnprotectedInitializer, late, (UNPROTECTED_INITIALIZER)),
    (Rtlo, early, (RTLO))
);
