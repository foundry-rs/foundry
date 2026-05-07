use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod block_timestamp;
use block_timestamp::BLOCK_TIMESTAMP;

mod missing_zero_check;
use missing_zero_check::MISSING_ZERO_CHECK;

register_lints!(
    (BlockTimestamp, early, (BLOCK_TIMESTAMP)),
    (MissingZeroCheck, late, (MISSING_ZERO_CHECK)),
);
