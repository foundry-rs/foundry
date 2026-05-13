use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod arbitrary_send_erc20;
mod incorrect_shift;
mod reentrancy;
mod rtlo;
mod unchecked_calls;

use arbitrary_send_erc20::ARBITRARY_SEND_ERC20;
use incorrect_shift::INCORRECT_SHIFT;
use reentrancy::REENTRANCY_UNLIMITED_GAS;
use rtlo::RTLO;
use unchecked_calls::{ERC20_UNCHECKED_TRANSFER, UNCHECKED_CALL};

register_lints!(
    (IncorrectShift, early, (INCORRECT_SHIFT)),
    (ReentrancyUnlimitedGas, late, (REENTRANCY_UNLIMITED_GAS)),
    (UncheckedCall, early, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER)),
    (ArbitrarySendErc20, late, (ARBITRARY_SEND_ERC20)),
    (Rtlo, early, (RTLO))
);
