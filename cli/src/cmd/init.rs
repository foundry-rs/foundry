//! init command

use super::install;
use crate::{cmd::Cmd, opts::forge::Dependency};
use clap::{Parser, ValueHint};
use foundry_config::Config;

use std::{path::PathBuf, process::Command, str::FromStr};

/// Command to initialize a new forge project
#[derive(Debug, Clone, Parser)]
pub struct InitArgs {
    #[clap(
    help = "the project's root path, default being the current working directory",
    value_hint = ValueHint::DirPath
    )]
    root: Option<PathBuf>,
    #[clap(help = "optional solidity template to start from", long, short)]
    template: Option<String>,
}

impl Cmd for InitArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let InitArgs { root, template } = self;

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
            std::fs::create_dir_all(&lib)?;

            // write the contract file
            let contract_path = src.join("Contract.sol");
            std::fs::write(contract_path, include_str!("../../../assets/ContractTemplate.sol"))?;
            // write the tests
            let contract_path = test.join("Contract.t.sol");
            std::fs::write(contract_path, include_str!("../../../assets/ContractTemplate.t.sol"))?;

            // sets up git
            let is_git = Command::new("git")
                .args(&["rev-parse", "--is-inside-work-tree"])
                .current_dir(&root)
                .spawn()?
                .wait()?;
            if !is_git.success() {
                let gitignore_path = root.join(".gitignore");
                std::fs::write(gitignore_path, include_str!("../../../assets/.gitignoreTemplate"))?;
                Command::new("git").arg("init").current_dir(&root).spawn()?.wait()?;
                Command::new("git").args(&["add", "."]).current_dir(&root).spawn()?.wait()?;
                Command::new("git")
                    .args(&["commit", "-m", "chore: forge init"])
                    .current_dir(&root)
                    .spawn()?
                    .wait()?;
            }
            Dependency::from_str("https://github.com/dapphub/ds-test")
                .and_then(|dependency| install(&root, vec![dependency]))?;

            let dest = root.join(Config::FILE_NAME);
            if !dest.exists() {
                // write foundry.toml
                let config = Config::load_with_root(&root).into_basic();
                std::fs::write(dest, config.to_string_pretty()?)?;
            }
        }

        println!("Done.");
        Ok(())
    }
}
