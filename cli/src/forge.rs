pub mod cmd;
mod opts;
mod utils;

use crate::cmd::Cmd;

use ethers::solc::{Project, ProjectPathsConfig};
use opts::forge::{Dependency, FullContractInfo, Opts, Subcommands};
use std::process::Command;

use clap::{IntoApp, Parser};
use clap_complete::generate;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    utils::subscriber();

    let opts = Opts::parse();
    match opts.sub {
        Subcommands::Test(cmd) => {
            let outcome = cmd.run()?;
            outcome.ensure_ok()?;
        }
        Subcommands::Build(cmd) => {
            cmd.run()?;
        }
        Subcommands::Run(cmd) => {
            cmd.run()?;
        }
        Subcommands::VerifyContract { contract, address, constructor_args } => {
            let FullContractInfo { path, name } = contract;
            let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
            rt.block_on(cmd::verify::run(path, name, address, constructor_args))?;
        }
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
        Subcommands::Install { dependencies } => {
            cmd::install(std::env::current_dir()?, dependencies)?;
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
            generate(shell, &mut Opts::into_app(), "forge", &mut std::io::stdout())
        }
        Subcommands::Clean { root } => {
            let root = root.unwrap_or_else(|| std::env::current_dir().unwrap());
            let paths = ProjectPathsConfig::builder().root(&root).build()?;
            let project = Project::builder().paths(paths).build()?;
            project.cleanup()?;
        }
        Subcommands::Snapshot(cmd) => {
            cmd.run()?;
        }
        Subcommands::Config(cmd) => {
            cmd.run()?;
        }
        Subcommands::Flatten(cmd) => {
            cmd.run()?;
        }
    }

    Ok(())
}

fn remove(root: impl AsRef<std::path::Path>, dependencies: Vec<Dependency>) -> eyre::Result<()> {
    let libs = std::path::Path::new("lib");
    let git_mod_libs = std::path::Path::new(".git/modules/lib");

    dependencies.iter().try_for_each(|dep| -> eyre::Result<_> {
        let path = libs.join(&dep.name);
        let git_mod_path = git_mod_libs.join(&dep.name);
        println!("Removing {} in {:?}, (url: {}, tag: {:?})", dep.name, path, dep.url, dep.tag);

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

        // tell git to discard the removal of the submodule
        Command::new("git").args(&["checkout", "--", "."]).current_dir(&root).spawn()?.wait()?;

        Ok(())
    })
}
