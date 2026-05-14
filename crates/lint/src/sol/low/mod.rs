use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod block_timestamp;
use block_timestamp::BLOCK_TIMESTAMP;

mod missing_zero_check;
use missing_zero_check::MISSING_ZERO_CHECK;

mod return_bomb;
use return_bomb::RETURN_BOMB;

register_lints!(
    (BlockTimestamp, early, (BLOCK_TIMESTAMP)),
    (MissingZeroCheck, late, (MISSING_ZERO_CHECK)),
    (ReturnBomb, late, (RETURN_BOMB)),
);
