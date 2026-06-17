use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod block_timestamp;
use block_timestamp::BLOCK_TIMESTAMP;

mod calls_loop;
use calls_loop::CALLS_LOOP;

mod delegatecall_loop;
use delegatecall_loop::DELEGATECALL_LOOP;

pub(crate) mod incorrect_modifier;
use incorrect_modifier::INCORRECT_MODIFIER;

mod msg_value_loop;
use msg_value_loop::MSG_VALUE_LOOP;

mod missing_zero_check;
use missing_zero_check::MISSING_ZERO_CHECK;

mod missing_events_access_control;
use missing_events_access_control::MISSING_EVENTS_ACCESS_CONTROL;

mod missing_events_arithmetic;
use missing_events_arithmetic::MISSING_EVENTS_ARITHMETIC;

mod return_bomb;
use return_bomb::RETURN_BOMB;

mod payable_loop;

mod reentrancy_events;
use reentrancy_events::REENTRANCY_EVENTS;

register_lints!(
    (BlockTimestamp, late, (BLOCK_TIMESTAMP)),
    (CallsLoop, late, (CALLS_LOOP)),
    (DelegatecallLoop, late, (DELEGATECALL_LOOP)),
    (IncorrectModifier, late, (INCORRECT_MODIFIER)),
    (MsgValueLoop, late, (MSG_VALUE_LOOP)),
    (MissingEventsAccessControl, late, (MISSING_EVENTS_ACCESS_CONTROL)),
    (MissingEventsArithmetic, late, (MISSING_EVENTS_ARITHMETIC)),
    (MissingZeroCheck, late, (MISSING_ZERO_CHECK)),
    (ReturnBomb, late, (RETURN_BOMB)),
    (ReentrancyEvents, late, (REENTRANCY_EVENTS)),
);
