use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod arbitrary_send_erc20;
mod incorrect_shift;
mod reentrancy;
mod rtlo;
mod unchecked_calls;
mod unprotected_initializer;

use arbitrary_send_erc20::ARBITRARY_SEND_ERC20;
use incorrect_shift::INCORRECT_SHIFT;
use reentrancy::REENTRANCY_ETH;
use rtlo::RTLO;
use unchecked_calls::{ERC20_UNCHECKED_TRANSFER, UNCHECKED_CALL};
use unprotected_initializer::UNPROTECTED_INITIALIZER;

register_lints!(
    (ArbitrarySendErc20, late, (ARBITRARY_SEND_ERC20)),
    (IncorrectShift, early, (INCORRECT_SHIFT)),
    (ReentrancyEth, late, (REENTRANCY_ETH)),
    (UncheckedCall, early, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER)),
    (UnprotectedInitializer, late, (UNPROTECTED_INITIALIZER)),
    (Rtlo, early, (RTLO))
);
