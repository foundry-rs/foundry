use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod block_timestamp;
use block_timestamp::BLOCK_TIMESTAMP;

mod calls_loop;
use calls_loop::CALLS_LOOP;

mod missing_zero_check;
use missing_zero_check::MISSING_ZERO_CHECK;

register_lints!(
    (BlockTimestamp, early, (BLOCK_TIMESTAMP)),
    (CallsLoop, late, (CALLS_LOOP)),
    (MissingZeroCheck, late, (MISSING_ZERO_CHECK)),
);
