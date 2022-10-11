use ansi_term::Color::{Green, Red};
use clap::Parser;
use rustyline::error::ReadlineError;
use std::rc::Rc;

use chisel::{
    dispatcher::ChiselCommand,
    session::{ChiselEnv, SolSnippet}, prelude::DispatchResult,
};

/// Chisel is a fast, utilitarian, and verbose solidity REPL.
#[derive(Debug, Parser)]
#[clap(name = "chisel", version = "v0.0.1-alpha")]
pub struct ChiselParser {
    /// Set the RPC URL to fork.
    #[clap(long, short)]
    pub fork_url: Option<String>,

    /// Set the solc version that the REPL environment will use.
    #[clap(long, short)]
    pub solc: Option<String>,
}

fn main() {
    // Parse command args
    let _args = ChiselParser::parse();

    // Set up default `ChiselEnv` Configuration
    let mut env = ChiselEnv::default();

    // Keeps track of whether or not an interrupt was the last input
    let mut interrupt = false;

    // Create a new rustyline Editor
    let rl = Editor::<()>::new().unwrap_or_else(|e| {
        tracing::error!(target: "chisel-env", "Failed to initialize rustyline Editor! {}", e);
        panic!("failed to create a rustyline Editor for the chisel environment! {e}");
    });

    // Create a new cli dispatcher
    let mut dispatcher = ChiselDisptacher::new();

    // Begin Rustyline loop
    loop {
        // Get the prompt from the dispatcher
        // Variable based on status of the last entry
        let prompt = dispatcher.get_prompt();

        // Read the next line
        let next_string = rl.readline(prompt.as_str());

        // Try to read the string
        match next_string {
            Ok(line) => {
                interrupt = false;
                // Dispatch and match results
                match dispatcher.dispatch(line) {
                    DispatchResult::Success(Some(msg))
                    | DispatchResult::CommandSuccess(Some(msg)) => println!("{}", Green.paint(msg)),
                    DispatchResult::UnrecognizedCommand(e) => eprintln!("{}", e),
                    DispatchResult::SolangParserFailed(e) => {
                        eprintln!("{}", Red.paint("Compilation error"));
                        eprintln!("{}", Red.paint(format!("{:?}", e)));
                    }
                    DispatchResult::Success(None) => { /* Do nothing */ }
                    DispatchResult::CommandSuccess(_) => { /* Don't need to do anything here */ }
                    DispatchResult::CommandFailed(msg) => eprintln!("{}", Red.paint(msg)),
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
