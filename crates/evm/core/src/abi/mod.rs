//! Several ABI-related utilities for executors.

pub use foundry_cheatcodes_spec::Vm;

mod console;
pub use console::Console;

mod hardhat_console;
pub use hardhat_console::{hh_console_selector, patch_hh_console_selector, HardhatConsole};
