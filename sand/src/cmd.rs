//! cairo-lang/starknet cli bindings

use std::process::Command;

pub struct CompileOptions {
    pub file: u32,
}

pub struct StarknetCli {}

/// Bindings for [cairo-lang-docker](https://github.com/Shard-Labs/cairo-cli-docker)
#[derive(Debug, Clone)]
pub struct StarknetDockerCli {}

fn compile(cmd: Command) {}
