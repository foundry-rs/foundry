use crate::env::ChiselEnv;
use clap::Parser;
use rustyline::error::ReadlineError;

/// REPL env.
pub mod env;

/// REPL command dispatcher.
pub mod cmd;

/// A module for highlighting Solidity code within the REPL
pub mod sol_highlighter;

/// Chisel is a fast, utilitarian, and verbose solidity REPL.
#[derive(Debug, Parser)]
#[clap(name = "chisel", version = "v0.0.1-alpha")]
pub struct ChiselCommand {
    /// Set the RPC URL to fork.
    #[clap(long, short)]
    pub fork_url: Option<String>,

    /// Set the solc version that the REPL environment will use.
    #[clap(long, short)]
    pub solc: Option<String>,
}

fn main() {
    // Parse command args
    let _args = ChiselCommand::parse();

    // Set up default `ChiselEnv`
    // TODO: Configuration etc.
    let mut env = ChiselEnv::default();

    // Begin Rustyline loop
    loop {
        let line = env.rl.readline(">> ");
        match line {
            Ok(line) => {
                // TODO compilation, error checking before committing addition to session, basically everything lmao.
                
                // Playing w/ `TempProject`...
                env.session.push(line);
                if env.project.add_source("REPL", env.contract_source()).is_ok() {
                    println!("{:?}", env.project.sources_path());
                } else {
                    eprintln!("Error writing source file to temp project.");
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                println!("Exiting chisel.");
                break
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break
            }
        }
    }
}
