use alloy_primitives::Address;
use clap::Parser;

/// Chisel REPL commands.
#[derive(Debug, Parser)]
pub enum ChiselCommand {
    /// Print helpful information about chisel.
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
    #[command(name = "clear", visible_alias = "ls")]
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

    /// Set calldata (`msg.data`) for the current session (appended after function selector).
    #[command(visible_alias = "cd")]
    Calldata {
        /// Calldata (empty to clear).
        data: Option<String>,
    },

    /// Dump the raw memory.
    #[command(visible_alias = "md", next_help_heading = "Debug")]
    MemDump,

    /// Dump the raw stack.
    #[command(visible_alias = "sd")]
    StackDump,

    /// Display the raw value of a variable's stack allocation.
    #[command(visible_alias = "rs")]
    RawStack {
        /// Variable name.
        var: String,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn see_help() {
        panic!("{}", ChiselCommand::command().render_help());
    }
}
