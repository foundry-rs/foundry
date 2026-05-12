use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod calls;
mod incorrect_shift;
mod low_level_calls;
mod rtlo;
mod unchecked_calls;

use incorrect_shift::INCORRECT_SHIFT;
use low_level_calls::LOW_LEVEL_CALLS;
use rtlo::RTLO;
use unchecked_calls::{ERC20_UNCHECKED_TRANSFER, UNCHECKED_CALL};

register_lints!(
    (IncorrectShift, early, (INCORRECT_SHIFT)),
    (LowLevelCalls, early, (LOW_LEVEL_CALLS)),
    (UncheckedCall, early, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER)),
    (Rtlo, early, (RTLO))
);
