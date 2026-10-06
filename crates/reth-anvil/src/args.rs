use crate::opts::{Anvil, AnvilSubcommand};
use clap::{CommandFactory, Parser};
use eyre::Result;
use foundry_cli::utils;

/// Run the `anvil` command line interface.
pub fn run() -> Result<()> {
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Anvil>();
    let warnings = utils::common_setup();
    let args = Anvil::parse();
    args.global.init()?;
    for warning in warnings {
        let _ = foundry_common::sh_warn!("{warning}");
    }
    run_command(args)
}

/// Run the subcommand.
pub fn run_command(args: Anvil) -> Result<()> {
    if let Some(cmd) = &args.cmd {
        match cmd {
            AnvilSubcommand::Completions { shell } => {
                clap_complete::generate(
                    *shell,
                    &mut Anvil::command(),
                    "anvil",
                    &mut std::io::stdout(),
                );
            }
        }
        return Ok(());
    }

    let _ = fdlimit::raise_fd_limit();
    args.global.tokio_runtime().block_on(args.node.run())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        Anvil::command().debug_assert();
    }

    #[test]
    fn can_parse_help() {
        let _: Anvil = Anvil::parse_from(["anvil", "--help"]);
    }

    #[test]
    fn can_parse_completions() {
        let args: Anvil = Anvil::parse_from(["anvil", "completions", "bash"]);
        assert!(matches!(
            args.cmd,
            Some(AnvilSubcommand::Completions {
                shell: foundry_cli::clap::Shell::ClapCompleteShell(clap_complete::Shell::Bash)
            })
        ));
    }
}
