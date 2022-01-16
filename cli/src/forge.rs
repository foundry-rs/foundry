use clap::{IntoApp, Parser};
use clap_complete::generate;
use std::{process::Command, str::FromStr};

use ethers::solc::{Project, ProjectPathsConfig};
use rayon::prelude::*;
use walkdir::WalkDir;

use forge_fmt::{Formatter, FormatterConfig, Visitable};
use opts::forge::{Dependency, FullContractInfo, Opts, Subcommands};

use crate::cmd::Cmd;

pub mod cmd;
mod opts;
mod utils;

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
            install(std::env::current_dir()?, dependencies)?;
        }
        Subcommands::Remove { dependencies } => {
            remove(std::env::current_dir()?, dependencies)?;
        }
        Subcommands::Remappings(cmd) => {
            cmd.run()?;
        }
        Subcommands::Init { root, template } => {
            let root = root.unwrap_or_else(|| std::env::current_dir().unwrap());
            // create the root dir if it does not exist
            if !root.exists() {
                std::fs::create_dir_all(&root)?;
            }
            let root = dunce::canonicalize(root)?;

            // if a template is provided, then this command is just an alias to `git clone <url>
            // <path>`
            if let Some(ref template) = template {
                println!("Initializing {} from {}...", root.display(), template);
                Command::new("git")
                    .args(&["clone", template, &root.display().to_string()])
                    .spawn()?
                    .wait()?;
            } else {
                println!("Initializing {}...", root.display());

                // make the dirs
                let src = root.join("src");
                let test = src.join("test");
                std::fs::create_dir_all(&test)?;
                let lib = root.join("lib");
                std::fs::create_dir(&lib)?;

                // write the contract file
                let contract_path = src.join("Contract.sol");
                std::fs::write(contract_path, include_str!("../../assets/ContractTemplate.sol"))?;
                // write the tests
                let contract_path = test.join("Contract.t.sol");
                std::fs::write(contract_path, include_str!("../../assets/ContractTemplate.t.sol"))?;

                // sets up git
                let is_git = Command::new("git")
                    .args(&["rev-parse", "--is-inside-work-tree"])
                    .current_dir(&root)
                    .spawn()?
                    .wait()?;
                if !is_git.success() {
                    let gitignore_path = root.join(".gitignore");
                    std::fs::write(
                        gitignore_path,
                        include_str!("../../assets/.gitignoreTemplate"),
                    )?;
                    Command::new("git").arg("init").current_dir(&root).spawn()?.wait()?;
                    Command::new("git").args(&["add", "."]).current_dir(&root).spawn()?.wait()?;
                    Command::new("git")
                        .args(&["commit", "-m", "chore: forge init"])
                        .current_dir(&root)
                        .spawn()?
                        .wait()?;
                }
                Dependency::from_str("https://github.com/dapphub/ds-test")
                    .and_then(|dependency| install(root, vec![dependency]))?;
            }

            println!("Done.");
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
        Subcommands::Fmt { path, root, check } => {
            if path.is_some() && root.is_some() {
                return Err(eyre::eyre!("Only --root or path argument should be specified"))
            }

            let root = if let Some(path) = path {
                path
            } else {
                let root = root.unwrap_or_else(|| {
                    std::env::current_dir().expect("failed to get current directory")
                });
                if !root.is_dir() {
                    return Err(eyre::eyre!("Root path should be a directory"))
                }

                ProjectPathsConfig::find_source_dir(&root)
            };

            let paths = if root.is_dir() {
                WalkDir::new(root)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".sol"))
                    .map(|e| e.into_path())
                    .collect()
            } else if root.file_name().unwrap().to_string_lossy().ends_with(".sol") {
                vec![root]
            } else {
                vec![]
            };

            paths.par_iter().map(|path| {
                let source = std::fs::read_to_string(&path)?;
                let mut source_unit = solang_parser::parse(&source, 0)
                    .map_err(|diags| eyre::eyre!(
                        "Failed to parse Solidity code for {}. Leave source unchanged.\nDebug info: {:?}",
                        path.to_string_lossy(),
                        diags
                    ))?;

                let mut output = String::new();
                let mut formatter =
                    Formatter::new(&mut output, &source, FormatterConfig::default());

                source_unit.visit(&mut formatter).unwrap();

                solang_parser::parse(&output, 0).map_err(|diags| {
                    eyre::eyre!(
                        "Failed to construct valid Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                        path.to_string_lossy(),
                        diags
                    )
                })?;

                if check {
                    if source != output {
                        std::process::exit(1);
                    }
                } else {
                    std::fs::write(path, output)?;
                }

                Ok(())
            }).collect::<eyre::Result<_>>()?;
        }
    }

    Ok(())
}

fn install(root: impl AsRef<std::path::Path>, dependencies: Vec<Dependency>) -> eyre::Result<()> {
    let libs = std::path::Path::new("lib");

    dependencies.iter().try_for_each(|dep| -> eyre::Result<_> {
        let path = libs.join(&dep.name);
        println!("Installing {} in {:?}, (url: {}, tag: {:?})", dep.name, path, dep.url, dep.tag);

        // install the dep
        Command::new("git")
            .args(&["submodule", "add", &dep.url, &path.display().to_string()])
            .current_dir(&root)
            .spawn()?
            .wait()?;

        // call update on it
        Command::new("git")
            .args(&["submodule", "update", "--init", "--recursive", &path.display().to_string()])
            .current_dir(&root)
            .spawn()?
            .wait()?;

        // checkout the tag if necessary
        let message = if let Some(ref tag) = dep.tag {
            Command::new("git")
                .args(&["checkout", "--recurse-submodules", tag])
                .current_dir(&path)
                .spawn()?
                .wait()?;

            Command::new("git").args(&["add", &path.display().to_string()]).spawn()?.wait()?;

            format!("forge install: {}\n\n{}", dep.name, tag)
        } else {
            format!("forge install: {}", dep.name)
        };

        Command::new("git").args(&["commit", "-m", &message]).current_dir(&root).spawn()?.wait()?;

        Ok(())
    })
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
