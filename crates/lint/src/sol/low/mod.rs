use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod block_timestamp;
use block_timestamp::BLOCK_TIMESTAMP;

mod calls_loop;
use calls_loop::CALLS_LOOP;

mod delegatecall_loop;
use delegatecall_loop::DELEGATECALL_LOOP;

mod missing_zero_check;
use missing_zero_check::MISSING_ZERO_CHECK;

mod return_bomb;
use return_bomb::RETURN_BOMB;

register_lints!(
    (BlockTimestamp, early, (BLOCK_TIMESTAMP)),
    (CallsLoop, late, (CALLS_LOOP)),
    (DelegatecallLoop, late, (DELEGATECALL_LOOP)),
    (MissingZeroCheck, late, (MISSING_ZERO_CHECK)),
    (ReturnBomb, late, (RETURN_BOMB)),
);
