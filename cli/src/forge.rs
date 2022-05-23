pub mod cmd;
pub mod compile;
mod opts;
mod suggestions;
mod term;
mod utils;

use crate::cmd::{
    forge::{cache::CacheSubcommands, watch},
    Cmd,
};
use opts::forge::{Dependency, Opts, Subcommands};
use std::process::Command;

use clap::{IntoApp, Parser};
use clap_complete::generate;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
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
        Subcommands::Run(cmd) => {
            cmd.run()?;
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
            cmd.run()?;
        }
        Subcommands::Update { lib } => {
            let mut cmd = Command::new("git");

            cmd.args(&["submodule", "update", "--remote", "--init", "--recursive"]);

            // if a lib is specified, open it
            if let Some(lib) = lib {
                cmd.args(&["--", lib.display().to_string().as_str()]);
            }

            cmd.spawn()?.wait()?;
        }
        // TODO: Make it work with updates?
        Subcommands::Install(cmd) => {
            cmd.run()?;
        }
        Subcommands::Remove { dependencies } => {
            remove(std::env::current_dir()?, dependencies)?;
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
        Subcommands::Tree(cmd) => {
            cmd.run()?;
        }
    }

    Ok(())
}

fn remove(root: impl AsRef<std::path::Path>, dependencies: Vec<Dependency>) -> eyre::Result<()> {
    let libs = std::path::Path::new("lib");
    let git_mod_root = std::path::Path::new(".git/modules");

    dependencies.iter().try_for_each(|dep| -> eyre::Result<_> {
        let target_dir = if let Some(alias) = &dep.alias { alias } else { &dep.name };
        let path = libs.join(&target_dir);
        let git_mod_path = git_mod_root.join(&path);
        println!("Removing {} in {:?}, (url: {:?}, tag: {:?})", dep.name, path, dep.url, dep.tag);

        // remove submodule entry from .git/config
        Command::new("git")
            .args(&["submodule", "deinit", "-f", &path.display().to_string()])
            .current_dir(&root)
            .spawn()?
            .wait()?;

        // remove the submodule repository from .git/modules directory
        Command::new("rm")
            .args(&["-rf", &git_mod_path.display().to_string()])
            .current_dir(&root)
            .spawn()?
            .wait()?;

        // remove the leftover submodule directory
        Command::new("git")
            .args(&["rm", "-f", &path.display().to_string()])
            .current_dir(&root)
            .spawn()?
            .wait()?;

        Ok(())
    })
}
