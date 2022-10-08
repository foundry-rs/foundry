use crate::env::ChiselEnv;
use ansi_term::Color::{Green, Red};
use clap::Parser;
use cmd::ChiselCommand;
use env::SolSnippet;
use rustyline::error::ReadlineError;
use std::rc::Rc;

/// REPL env.
pub mod env;

/// REPL command dispatcher.
pub mod cmd;

/// A module for highlighting Solidity code within the REPL
pub mod sol_highlighter;

/// Prompt arrow slice
static PROMPT_ARROW: &str = "➜ ";
/// Command leader character
static COMMAND_LEADER: char = '!';

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

    // Keeps track of whether or not the last input resulted in an error
    // TODO: This will probably best be tracked in the `ChiselEnv`,
    // just for mocking up the project.
    let mut error = false;

    // Begin Rustyline loop
    loop {
        let prompt =
            format!("{}", if error { Red.paint(PROMPT_ARROW) } else { Green.paint(PROMPT_ARROW) });

        match env.rl.readline(prompt.as_str()) {
            Ok(line) => {
                // Check if the input is a builtin command.
                // Commands are denoted with a `!` leading character.
                if line.starts_with(COMMAND_LEADER) {
                    let split: Vec<&str> = line.split(' ').collect();
                    let raw_cmd = &split[0][1..];

                    match raw_cmd.parse::<ChiselCommand>() {
                        Ok(cmd) => {
                            // TODO: Move `error` to the `ChiselEnv`, dispatch
                            // could still result in an error.
                            error = false;
                            cmd.dispatch(&split[1..], &mut env);
                        }
                        Err(e) => {
                            error = true;
                            eprintln!("{}", e);
                        }
                    }
                    continue
                }

                // Parse the input with [solang-parser](https://docs.rs/solang-parser/latest/solang_parser)
                // Print dianostics and continue on error
                // If parsing successful, grab the (source unit, comment) tuple
                //
                // TODO: This does check if the line is parsed successfully, but does
                // not check if the line conflicts with any previous declarations
                // (i.e. "uint a = 1;" could be declared twice). Should check against
                // the whole temp file so that previous inputs persist.
                let parsed = match solang_parser::parse(&line, 0) {
                    Ok(su) => su,
                    Err(e) => {
                        eprintln!("{}", Red.paint("Compilation error"));
                        eprintln!("{}", Red.paint(format!("{:?}", e)));
                        error = true;
                        continue
                    }
                };

                // Reset interrupt flag
                interrupt = false;
                // Reset error flag
                error = false;

                // Push the parsed source unit and comments to the environment session
                env.session.push(SolSnippet { source_unit: parsed, raw: Rc::new(line) });
                if env.project.add_source("REPL", env.contract_source()).is_ok() {
                    if env.run_repl().is_err() {
                        eprintln!("{}", Red.paint("Compilation error"));

                        // Remove line that caused the compilation error
                        env.session.pop();
                    }
                    // println!("{:?}", env.project.sources_path());
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
