use crate::opts::{Anvil, AnvilSubcommand};
use clap::{CommandFactory, Parser};
use eyre::Result;
use foundry_cli::{handler, utils};

/// Run the `anvil` command line interface.
pub fn run() -> Result<()> {
    setup()?;

    let mut args = Anvil::parse();
    args.global.init()?;
    args.node.evm.resolve_rpc_alias();

    run_command(args)
}

/// Setup the exception handler and other utilities.
pub fn setup() -> Result<()> {
    utils::install_crypto_provider();
    handler::install();
    utils::load_dotenv();
    utils::enable_paint();

    Ok(())
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
            AnvilSubcommand::GenerateFigSpec => clap_complete::generate(
                clap_complete_fig::Fig,
                &mut Anvil::command(),
                "anvil",
                &mut std::io::stdout(),
            ),
        }
        return Ok(());
    }

    let _ = fdlimit::raise_fd_limit();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(args.node.run())
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
    fn can_parse_short_version() {
        let _: Anvil = Anvil::parse_from(["anvil", "-V"]);
    }

    #[test]
    fn can_parse_long_version() {
        let _: Anvil = Anvil::parse_from(["anvil", "--version"]);
    }

    #[test]
    fn can_parse_completions() {
        let args: Anvil = Anvil::parse_from(["anvil", "completions", "bash"]);
        assert!(matches!(
            args.cmd,
            Some(AnvilSubcommand::Completions { shell: clap_complete::Shell::Bash })
        ));
    }
}
