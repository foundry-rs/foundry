pub mod cmd;
pub mod compile;
mod handler;
mod opts;
mod suggestions;
mod term;
mod utils;

use crate::{
    cmd::{
        forge::{cache::CacheSubcommands, watch},
        Cmd,
    },
    utils::CommandUtils,
};
use clap::{IntoApp, Parser};
use clap_complete::generate;
use opts::forge::{Opts, Subcommands};
use std::process::Command;

fn main() -> eyre::Result<()> {
    handler::install()?;
    utils::subscriber();
    utils::enable_paint();

    let opts = Opts::parse();
    match opts.sub {
        Subcommands::Test(cmd) => {
            if cmd.is_watch() {
                utils::block_on(watch::watch_test(cmd))?;
            } else {
                let outcome = cmd.run()?;
                outcome.ensure_ok()?;
            }
        }
        Subcommands::Script(cmd) => {
            utils::block_on(cmd.run_script())?;
        }
        Subcommands::Coverage(cmd) => {
            cmd.run()?;
        }
        Subcommands::Bind(cmd) => {
            cmd.run()?;
        }
        Subcommands::Build(cmd) => {
            if cmd.is_watch() {
                utils::block_on(crate::cmd::forge::watch::watch_build(cmd))?;
            } else {
                cmd.run()?;
            }
        }
        Subcommands::Debug(cmd) => {
            utils::block_on(cmd.debug())?;
        }
        Subcommands::VerifyContract(args) => {
            utils::block_on(args.run())?;
        }
        Subcommands::VerifyCheck(args) => {
            utils::block_on(args.run())?;
        }
        Subcommands::Cache(cmd) => match cmd.sub {
            CacheSubcommands::Clean(cmd) => {
                cmd.run()?;
            }
            CacheSubcommands::Ls(cmd) => {
                cmd.run()?;
            }
        },
        Subcommands::Create(cmd) => {
            utils::block_on(cmd.run())?;
        }
        Subcommands::Update { lib } => {
            let mut cmd = Command::new("git");

            cmd.args(&["submodule", "update", "--remote", "--init", "--recursive"]);

            // if a lib is specified, open it
            if let Some(lib) = lib {
                cmd.args(&["--", lib.display().to_string().as_str()]);
            }

            cmd.exec()?;
        }
        // TODO: Make it work with updates?
        Subcommands::Install(cmd) => {
            cmd.run()?;
        }
        Subcommands::Remove(cmd) => {
            cmd.run()?;
        }
        Subcommands::Remappings(cmd) => {
            cmd.run()?;
        }
        Subcommands::Init(cmd) => {
            cmd.run()?;
        }
        Subcommands::Completions { shell } => {
            generate(shell, &mut Opts::command(), "forge", &mut std::io::stdout())
        }
        Subcommands::Clean { root } => {
            let config = utils::load_config_with_root(root);
            config.project()?.cleanup()?;
        }
        Subcommands::Snapshot(cmd) => {
            if cmd.is_watch() {
                utils::block_on(crate::cmd::forge::watch::watch_snapshot(cmd))?;
            } else {
                cmd.run()?;
            }
        }
        Subcommands::Fmt(cmd) => {
            cmd.run()?;
        }
        Subcommands::Config(cmd) => {
            cmd.run()?;
        }
        Subcommands::Flatten(cmd) => {
            cmd.run()?;
        }
        Subcommands::Inspect(cmd) => {
            cmd.run()?;
        }
        Subcommands::UploadSelectors(args) => {
            utils::block_on(args.run())?;
        }
        Subcommands::Tree(cmd) => {
            cmd.run()?;
        }
    }

    Ok(())
}
