use crate::sol::{EarlyLintPass, LateLintPass, SolLint};

mod arbitrary_send_erc20;
mod arbitrary_send_eth;
mod controlled_delegatecall;
mod encode_packed_collision;
mod enumerable_loop_removal;
mod incorrect_exp;
mod incorrect_shift;
mod reentrancy;
mod rtlo;
mod unchecked_calls;
mod unprotected_initializer;

use arbitrary_send_erc20::{ARBITRARY_SEND_ERC20, ARBITRARY_SEND_ERC20_PERMIT};
use arbitrary_send_eth::ARBITRARY_SEND_ETH;
use controlled_delegatecall::CONTROLLED_DELEGATECALL;
use encode_packed_collision::ENCODE_PACKED_COLLISION;
use enumerable_loop_removal::ENUMERABLE_LOOP_REMOVAL;
use incorrect_exp::INCORRECT_EXP;
use incorrect_shift::INCORRECT_SHIFT;
use reentrancy::{REENTRANCY_ETH, REENTRANCY_NO_ETH};
use rtlo::RTLO;
use unchecked_calls::{ERC20_UNCHECKED_TRANSFER, UNCHECKED_CALL};
use unprotected_initializer::UNPROTECTED_INITIALIZER;

register_lints!(
    (ArbitrarySendErc20, late, (ARBITRARY_SEND_ERC20, ARBITRARY_SEND_ERC20_PERMIT)),
    (ArbitrarySendEth, late, (ARBITRARY_SEND_ETH)),
    (ControlledDelegatecall, late, (CONTROLLED_DELEGATECALL)),
    (EncodedPackedCollision, late, (ENCODE_PACKED_COLLISION)),
    (EnumerableLoopRemoval, late, (ENUMERABLE_LOOP_REMOVAL)),
    (IncorrectExp, late, (INCORRECT_EXP)),
    (IncorrectShift, early, (INCORRECT_SHIFT)),
    (ReentrancyEth, late, (REENTRANCY_ETH, REENTRANCY_NO_ETH)),
    (UncheckedCall, early, (UNCHECKED_CALL)),
    (UncheckedTransferERC20, late, (ERC20_UNCHECKED_TRANSFER)),
    (UnprotectedInitializer, late, (UNPROTECTED_INITIALIZER)),
    (Rtlo, early, (RTLO))
);
