use crate::prelude::CHISEL_CHAR;
use alloy_primitives::Address;
use clap::{CommandFactory, Parser};
use itertools::Itertools;
use yansi::Paint;

/// Chisel REPL commands.
#[derive(Debug, Parser)]
#[command(disable_help_flag = true, disable_help_subcommand = true)]
pub enum ChiselCommand {
    /// Display all commands.
    #[command(visible_alias = "h", next_help_heading = "General")]
    Help,

    /// Quit the REPL.
    #[command(visible_alias = "q")]
    Quit,

    /// Executes a shell command.
    #[command(visible_alias = "e")]
    Exec {
        /// Command to execute.
        command: String,
        /// Command arguments.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Clear the current session source.
    #[command(visible_alias = "c", next_help_heading = "Session")]
    Clear,

    /// Print the generated source contract.
    #[command(visible_alias = "so")]
    Source,

    /// Save the current session to the cache.
    #[command(visible_alias = "s")]
    Save {
        /// Optional session ID.
        id: Option<String>,
    },

    /// Load a previous session from cache.
    /// WARNING: This will overwrite the current session (though the current session will be
    /// optimistically cached).
    #[command(visible_alias = "l")]
    Load {
        /// Session ID to load.
        id: String,
    },

    /// List all cached sessions.
    #[command(name = "list", visible_alias = "ls")]
    ListSessions,

    /// Clear the cache of all stored sessions.
    #[command(name = "clearcache", visible_alias = "cc")]
    ClearCache,

    /// Export the current REPL session source to a Script file.
    #[command(visible_alias = "ex")]
    Export,

    /// Fetch an interface of a verified contract on Etherscan.
    #[command(visible_alias = "fe")]
    Fetch {
        /// Contract address.
        addr: Address,
        /// Interface name.
        name: String,
    },

    /// Open the current session in an editor.
    Edit,

    /// Fork an RPC in the current session.
    #[command(visible_alias = "f", next_help_heading = "Environment")]
    Fork {
        /// Fork URL, environment variable, or RPC endpoints alias (empty to return to local
        /// network).
        url: Option<String>,
    },

    /// Enable / disable traces for the current session.
    #[command(visible_alias = "t")]
    Traces,

    /// Set calldata (`msg.data`) for the current session (appended after function selector). Clears
    /// it if no argument provided.
    #[command(visible_alias = "cd")]
    Calldata {
        /// Calldata (empty to clear).
        data: Option<String>,
    },

    /// Dump the raw memory.
    #[command(name = "memdump", visible_alias = "md", next_help_heading = "Debug")]
    MemDump,

    /// Dump the raw stack.
    #[command(name = "stackdump", visible_alias = "sd")]
    StackDump,

    /// Display the raw value of a variable's stack allocation. For variables that are > 32 bytes in
    /// length, this will display their memory pointer.
    #[command(name = "rawstack", visible_alias = "rs")]
    RawStack {
        /// Variable name.
        var: String,
    },
}

impl ChiselCommand {
    pub fn parse(input: &str) -> eyre::Result<Self> {
        let args = input.split_whitespace();
        let args = std::iter::once("chisel").chain(args);
        Self::try_parse_from(args)
            .map_err(|e| eyre::eyre!("{}; for more information, see `!help`", e.kind()))
    }

    pub fn format_help() -> String {
        let cmd = Self::command();
        let mut categories = Vec::new();
        let mut cat = None;
        for sub in cmd.get_subcommands() {
            if let Some(cat_) = sub.get_next_help_heading()
                && Some(cat_) != cat
            {
                cat = Some(cat_);
                categories.push((cat_, vec![]));
            }
            categories.last_mut().unwrap().1.push(sub);
        }
        format!(
            "{}\n{}",
            format!("{CHISEL_CHAR} Chisel help\n=============").cyan(),
            categories
                .iter()
                .map(|(cat, cat_cmds)| {
                    format!(
                        "{}\n{}\n",
                        cat.magenta(),
                        cat_cmds
                            .iter()
                            .map(|&cmd| format!(
                                "\t{}{} - {}",
                                std::iter::once(cmd.get_name())
                                    .chain(cmd.get_visible_aliases())
                                    .map(|s| format!("!{}", s.green()))
                                    .format(" | "),
                                {
                                    let usage = get_usage(cmd);
                                    if usage.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" {usage}")
                                    }
                                }
                                .green(),
                                cmd.get_about().expect("command is missing about"),
                            ))
                            .format("\n")
                    )
                })
                .format("\n")
        )
    }
}

fn get_usage(cmd: &clap::Command) -> String {
    let s = cmd.clone().render_usage().to_string();
    if let Some(idx) = s.find(['[', '<']) {
        return s[idx..].to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_help() {
        let _ = sh_eprintln!("{}", ChiselCommand::format_help());
    }
}
