//! Several ABI-related utilities for executors.

pub use foundry_cheatcodes_spec::Vm;

mod console;
pub use console::Console;

mod hardhat_console;
pub use hardhat_console::{patch_hardhat_console_selector, HardhatConsole};
