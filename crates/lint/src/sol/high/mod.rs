use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

mod incorrect_shift;
mod unchecked_calls;

use incorrect_shift::INCORRECT_SHIFT;
use unchecked_calls::{UNCHECKED_CALL, ERC20_UNCHECKED_TRANSFER};

register_lints!(
    (IncorrectShift, (INCORRECT_SHIFT)),
    (UncheckedCall, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, (ERC20_UNCHECKED_TRANSFER))
);
