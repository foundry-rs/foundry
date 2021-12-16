use crate::Shell;
use log::debug;
use solang::parser::pt::*;

/// A user input
#[derive(Debug)]
pub enum Input {
    /// A known command with its shellwords
    Command(Command, Vec<String>),
    /// A deserialized solidity source unit
    Solang(SourceUnit),
    /// Unmatched input
    Other(String),
}

impl Input {
    /// Consumes the line
    pub fn read_line(line: impl Into<String>, shell: &mut Shell) -> Option<Self> {
        let line = line.into();
        if line.is_empty() {
            None
        } else {
            debug!("Readline returned {:?}", line);

            shell.rl.add_history_entry(line.as_str());

            if line.starts_with('#') {
                // shell comment
                return None
            }

            let words = match shellwords::split(&line) {
                Ok(cmd) => cmd,
                Err(err) => {
                    eprintln!("Error: {:?}", err);
                    return None
                }
            };
            debug!("shellwords output: {:?}", words);

            if words.is_empty() {
                return None
            }

            // try to find a matching native command
            if let Some(cmd) = Command::from_str(&words[0]) {
                return Some(Input::Command(cmd, words))
            }

            // try to deserialize as supported solidity command
            if let Ok(unit) = solang::parser::parse(&line, 1) {
                return Some(Input::Solang(unit))
            }

            // return unresolved content, delegate eval to shell itself, like `msg.sender`
            Some(Input::Other(line))
        }
    }
}

/// Various supported solang elements
#[derive(Debug)]
pub enum SolangInput {
    Contract(Box<ContractDefinition>),
    Function(Box<FunctionDefinition>),
    Variable(Box<VariableDefinition>),
    Struct(Box<StructDefinition>),
    // TODO various math expressions
}

/// various sol shell commands
#[derive(Debug)]
pub enum Command {
    Dump,
    Load,
    Help,
    Quit,
    Exit,
    Set,
    List,
    Interrupt,
}

impl Command {
    fn from_str(s: &str) -> Option<Command> {
        match s {
            "dump" => Some(Command::Dump),
            "load" => Some(Command::Load),
            "help" => Some(Command::Help),
            "quit" => Some(Command::Quit),
            "exit" => Some(Command::Exit),
            "set" => Some(Command::Set),
            "ls" | "list" => Some(Command::List),
            _ => None,
        }
    }
}
