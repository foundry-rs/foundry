use crate::opts::{Anvil, AnvilSubcommand};
use clap::{CommandFactory, Parser};
use eyre::Result;
use foundry_cli::utils;

/// Run the `anvil` command line interface.
pub fn run() -> Result<()> {
    // Pre-parse discovery flags run before `setup()` so they cannot be blocked
    // by panic-handler / tracing init failures and avoid that init's cost.
    foundry_cli::opts::GlobalArgs::check_introspect::<Anvil>();
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Anvil>();

    setup()?;

    let mut args = Anvil::parse();
    args.global.init()?;
    args.node.evm.resolve_rpc_alias();

    run_command(args)
}

/// Setup the exception handler and other utilities.
pub fn setup() -> Result<()> {
    utils::common_setup();

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
            Some(AnvilSubcommand::Completions {
                shell: foundry_cli::clap::Shell::ClapCompleteShell(clap_complete::Shell::Bash)
            })
        ));
    }

    /// Every `command_id` exposed by `anvil --introspect` MUST be unique.
    #[test]
    fn introspect_command_ids_are_unique() {
        use foundry_cli::introspect::{CommandRegistry, build_document, duplicate_command_ids};
        let cmd = <Anvil as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let dups = duplicate_command_ids(&doc);
        assert!(dups.is_empty(), "duplicate anvil command_ids: {dups:?}");
    }

    /// `anvil --introspect` must produce a JSON document that parses back into
    /// the canonical `IntrospectDocument` shape.
    #[test]
    fn introspect_document_is_valid_json() {
        use foundry_cli::introspect::{
            CommandRegistry, INTROSPECT_SCHEMA_ID, IntrospectDocument, render_introspect_document,
        };
        let cmd = <Anvil as clap::CommandFactory>::command();
        let json = render_introspect_document(&cmd, &CommandRegistry::EMPTY);
        let doc: IntrospectDocument = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(doc.schema_id, INTROSPECT_SCHEMA_ID);
        assert_eq!(doc.binary.name, "anvil");
    }
}
