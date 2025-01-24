//! Hardhat `console.sol` interface.

use alloy_sol_types::sol;
use foundry_common_fmt::*;
use foundry_macros::ConsoleFmt;

sol!(
    #[sol(abi)]
    #[derive(ConsoleFmt)]
    Console,
    "src/Console.json"
);

pub use Console::*;
