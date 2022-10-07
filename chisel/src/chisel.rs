use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::{Editor, Result};

/// REPL command dispatcher.
pub mod cmd;

/// A module for highlighting Solidity code within the REPL
pub mod sol_highlighter;

/// Chisel is a fast, utilitarian, and verbose solidity REPL.
#[derive(Debug, Parser)]
#[clap(name = "chisel", version = "v0.0.1-alpha")]
pub struct Chisel {
    /// Set the RPC URL to fork.
    #[clap(long, short)]
    pub fork_url: Option<String>,

    /// Set the solc version that the REPL environment will use.
    #[clap(long, short)]
    pub solc: Option<String>,
}

fn main() -> Result<()> {
    // Parse command args
    let _args = Chisel::parse();

    // Create new Rustyline editor
    let mut rl = Editor::<()>::new()?;

    // Begin Rustyline loop
    loop {
        let line = rl.readline(">> ");
        match line {
            Ok(line) => {
                // TODO
                println!("{}", line);
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

    Ok(())
}
