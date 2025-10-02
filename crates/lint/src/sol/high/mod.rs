use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod incorrect_shift;
mod unchecked_calls;

use incorrect_shift::INCORRECT_SHIFT;
use unchecked_calls::{ERC20_UNCHECKED_TRANSFER, UNCHECKED_CALL};

register_lints!(
    (IncorrectShift, early, (INCORRECT_SHIFT)),
    (UncheckedCall, early, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER))
);
