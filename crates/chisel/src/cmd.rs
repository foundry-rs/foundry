//! ChiselCommand
//!
//! This module holds the [ChiselCommand] enum, which contains all builtin commands that
//! can be executed within the REPL.

use crate::prelude::ChiselDispatcher;
use std::{error::Error, str::FromStr};
use strum::EnumIter;

/// Builtin chisel command variants
#[derive(Debug, EnumIter)]
pub enum ChiselCommand {
    /// Print helpful information about chisel
    Help,
    /// Quit the REPL
    Quit,
    /// Clear the current session source
    Clear,
    /// Print the generated source contract
    Source,
    /// Save the current session to the cache
    /// Takes: `<session-id>`
    Save,
    /// Load a previous session from cache
    /// Takes: `<session-id>`
    ///
    /// WARNING: This will overwrite the current session (though the current session will be
    /// optimistically cached)
    Load,
    /// List all cached sessions
    ListSessions,
    /// Clear the cache of all stored sessions
    ClearCache,
    /// Fork an RPC in the current session
    /// Takes <fork-url|env-var|rpc_endpoints-alias>
    Fork,
    /// Enable / disable traces for the current session
    Traces,
    /// Set calldata (`msg.data`) for the current session (appended after function selector)
    Calldata,
    /// Dump the raw memory
    MemDump,
    /// Dump the raw stack
    StackDump,
    /// Export the current REPL session source to a Script file
    Export,
    /// Fetch an interface of a verified contract on Etherscan
    /// Takes: `<addr> <interface-name>`
    Fetch,
    /// Executes a shell command
    Exec,
    /// Display the raw value of a variable's stack allocation.
    RawStack,
    /// Open the current session in an editor
    Edit,
}

/// Attempt to convert a string slice to a `ChiselCommand`
impl FromStr for ChiselCommand {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "help" | "h" => Ok(Self::Help),
            "quit" | "q" => Ok(Self::Quit),
            "clear" | "c" => Ok(Self::Clear),
            "source" | "so" => Ok(Self::Source),
            "save" | "s" => Ok(Self::Save),
            "list" | "ls" => Ok(Self::ListSessions),
            "load" | "l" => Ok(Self::Load),
            "clearcache" | "cc" => Ok(Self::ClearCache),
            "fork" | "f" => Ok(Self::Fork),
            "traces" | "t" => Ok(Self::Traces),
            "calldata" | "cd" => Ok(Self::Calldata),
            "memdump" | "md" => Ok(Self::MemDump),
            "stackdump" | "sd" => Ok(Self::StackDump),
            "export" | "ex" => Ok(Self::Export),
            "fetch" | "fe" => Ok(Self::Fetch),
            "exec" | "e" => Ok(Self::Exec),
            "rawstack" | "rs" => Ok(Self::RawStack),
            "edit" => Ok(Self::Edit),
            _ => Err(ChiselDispatcher::make_error(format!(
                "Unknown command \"{s}\"! See available commands with `!help`.",
            ))
            .into()),
        }
    }
}

/// A category for [ChiselCommand]s
#[derive(Debug, EnumIter)]
pub enum CmdCategory {
    /// General category
    General,
    /// Session category
    Session,
    /// Environment category
    Env,
    /// Debug category
    Debug,
}

impl core::fmt::Display for CmdCategory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let string = match self {
            Self::General => "General",
            Self::Session => "Session",
            Self::Env => "Environment",
            Self::Debug => "Debug",
        };
        f.write_str(string)
    }
}

/// A command descriptor type
pub type CmdDescriptor = (&'static [&'static str], &'static str, CmdCategory);

/// Convert a `ChiselCommand` into a `CmdDescriptor` tuple
impl From<ChiselCommand> for CmdDescriptor {
    fn from(cmd: ChiselCommand) -> Self {
        match cmd {
            // General
            ChiselCommand::Help => (&["help", "h"], "Display all commands", CmdCategory::General),
            ChiselCommand::Quit => (&["quit", "q"], "Quit Chisel", CmdCategory::General),
            ChiselCommand::Exec => (&["exec <command> [args]", "e <command> [args]"], "Execute a shell command and print the output", CmdCategory::General),
            // Session
            ChiselCommand::Clear => (&["clear", "c"], "Clear current session source", CmdCategory::Session),
            ChiselCommand::Source => (&["source", "so"], "Display the source code of the current session", CmdCategory::Session),
            ChiselCommand::Save => (&["save [id]", "s [id]"], "Save the current session to cache", CmdCategory::Session),
            ChiselCommand::Load => (&["load <id>", "l <id>"], "Load a previous session ID from cache", CmdCategory::Session),
            ChiselCommand::ListSessions => (&["list", "ls"], "List all cached sessions", CmdCategory::Session),
            ChiselCommand::ClearCache => (&["clearcache", "cc"], "Clear the chisel cache of all stored sessions", CmdCategory::Session),
            ChiselCommand::Export => (&["export", "ex"], "Export the current session source to a script file", CmdCategory::Session),
            ChiselCommand::Fetch => (&["fetch <addr> <name>", "fe <addr> <name>"], "Fetch the interface of a verified contract on Etherscan", CmdCategory::Session),
            // Environment
            ChiselCommand::Fork => (&["fork <url>", "f <url>"], "Fork an RPC for the current session. Supply 0 arguments to return to a local network", CmdCategory::Env),
            ChiselCommand::Traces => (&["traces", "t"], "Enable / disable traces for the current session", CmdCategory::Env),
            ChiselCommand::Calldata => (&["calldata [data]", "cd [data]"], "Set calldata (`msg.data`) for the current session (appended after function selector). Clears it if no argument provided.", CmdCategory::Env),
            // Debug
            ChiselCommand::MemDump => (&["memdump", "md"], "Dump the raw memory of the current state", CmdCategory::Debug),
            ChiselCommand::StackDump => (&["stackdump", "sd"], "Dump the raw stack of the current state", CmdCategory::Debug),
            ChiselCommand::Edit => (&["edit"], "Open the current session in an editor", CmdCategory::Session),
            ChiselCommand::RawStack => (&["rawstack <var>", "rs <var>"], "Display the raw value of a variable's stack allocation. For variables that are > 32 bytes in length, this will display their memory pointer.", CmdCategory::Debug),
        }
    }
}
