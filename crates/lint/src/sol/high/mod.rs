use crate::{
    register_lints,
    sol::{EarlyLintPass, SolLint},
};

mod incorrect_shift;
mod unchecked_call;
mod unchecked_transfer_erc20;

use incorrect_shift::INCORRECT_SHIFT;
use unchecked_call::UNCHECKED_CALL;
use unchecked_transfer_erc20::UNCHECKED_TRANSFER_ERC20;

register_lints!(
    (IncorrectShift, (INCORRECT_SHIFT)),
    (UncheckedCall, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, (UNCHECKED_TRANSFER_ERC20))
);
