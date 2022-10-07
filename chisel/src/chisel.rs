use crate::env::ChiselEnv;
use ansi_term::Color::{Green, Red};
use clap::Parser;
use rustyline::error::ReadlineError;

/// REPL env.
pub mod env;

/// REPL command dispatcher.
pub mod cmd;

/// A module for highlighting Solidity code within the REPL
pub mod sol_highlighter;

/// Prompt arrow slice
static PROMPT_ARROW: &str = "➜ ";

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

    // Keeps track of whether or not an interrupt was the last input
    let mut interrupt = false;

    // Keeps track of whether or not the last input resulted in an error
    // TODO: This will probably best be tracked in the `ChiselEnv`,
    // just for mocking up the project.
    #[allow(unused_mut)]
    let mut error = false;

    // Begin Rustyline loop
    loop {
        let prompt =
            format!("{}", if error { Red.paint(PROMPT_ARROW) } else { Green.paint(PROMPT_ARROW) });

        match env.rl.readline(prompt.as_str()) {
            Ok(line) => {
                // TODO compilation, error checking before committing addition to session, basically
                // everything lmao.
                
                if interrupt {
                    interrupt = false;
                }

                // Playing w/ `TempProject`...
                env.session.push(line);
                if env.project.add_source("REPL", env.contract_source()).is_ok() {
                    println!("{:?}", env.project.sources_path());
                } else {
                    eprintln!("Error writing source file to temp project.");
                }
            }
            Err(ReadlineError::Interrupted) => {
                if interrupt {
                    break
                } else {
                    println!("(To exit, press Ctrl+C again)");
                    interrupt = true;
                }
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                println!("Error: {:?}", err);
                break
            }
        }
    }
}
