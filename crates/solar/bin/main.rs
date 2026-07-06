//! The `solar` CLI: a Solidity compiler.

use solar_cli::{parse_args, run_compiler_args, signal_handler, utils};
use std::process::ExitCode;

#[global_allocator]
static ALLOC: utils::Allocator = utils::new_allocator();

fn main() -> ExitCode {
    signal_handler::install();
    let _guard = utils::init_logger(Default::default());
    let args = match parse_args(std::env::args_os()) {
        Ok(args) => args,
        Err(err) => err.exit(),
    };
    match run_compiler_args(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
