//! init command

use crate::{
    cmd::{
        forge::install::{ensure_git_status_clean, install, DependencyInstallOpts},
        Cmd,
    },
    opts::forge::Dependency,
    utils::{p_println, CommandUtils},
};
use clap::{Parser, ValueHint};
use ethers::solc::remappings::Remapping;
use foundry_common::fs;
use foundry_config::Config;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
};
use yansi::Paint;

/// Command to initialize a new forge project
#[derive(Debug, Clone, Parser)]
pub struct InitArgs {
    #[clap(
        help = "The root directory of the new project. Defaults to the current working directory.",
        value_hint = ValueHint::DirPath,
        value_name = "ROOT"
    )]
    root: Option<PathBuf>,
    #[clap(help = "The template to start from.", long, short, value_name = "TEMPLATE")]
    template: Option<String>,
    #[clap(help = "Do not create a git repository.", conflicts_with = "template", long)]
    no_git: bool,
    #[clap(help = "Do not create an initial commit.", conflicts_with = "template", long)]
    no_commit: bool,
    #[clap(help = "Do not print any messages.", short, long)]
    quiet: bool,
    #[clap(
        help = "Do not install dependencies from the network.",
        conflicts_with = "template",
        long,
        visible_alias = "no-deps"
    )]
    offline: bool,
    #[clap(
        help = "Create the project even if the specified root directory is not empty.",
        conflicts_with = "template",
        long
    )]
    force: bool,
    #[clap(
        help = "Create a .vscode/settings.json file with Solidity settings, and generate a remappings.txt file.",
        conflicts_with = "template",
        long
    )]
    vscode: bool,
}

impl Cmd for InitArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let InitArgs { root, template, no_git, no_commit, quiet, offline, force, vscode } = self;

        let root = root.unwrap_or_else(|| std::env::current_dir().unwrap());
        // create the root dir if it does not exist
        if !root.exists() {
            fs::create_dir_all(&root)?;
        }
        let root = dunce::canonicalize(root)?;

        // if a template is provided, then this command is just an alias to `git clone <url>
        // <path>`
        if let Some(template) = template {
            let template = if template.starts_with("https://") {
                template
            } else {
                "https://github.com/".to_string() + &template
            };
            p_println!(!quiet => "Initializing {} from {}...", root.display(), template);
            Command::new("git")
                .args(&["clone", "--recursive", &template, &root.display().to_string()])
                .exec()?;
        } else {
            // check if target is empty
            if !force && root.read_dir().map(|mut i| i.next().is_some()).unwrap_or(false) {
                eprintln!(
                    r#"{}: `forge init` cannot be run on a non-empty directory.

        run `forge init --force` to initialize regardless."#,
                    Paint::red("error")
                );
                std::process::exit(1);
            }

            // ensure git status is clean before generating anything
            if !no_git && !no_commit && is_git(&root)? {
                ensure_git_status_clean(&root)?;
            }

            p_println!(!quiet => "Initializing {}...", root.display());

            // make the dirs
            let src = root.join("src");
            fs::create_dir_all(&src)?;

            let test = root.join("test");
            fs::create_dir_all(&test)?;

            let script = root.join("script");
            fs::create_dir_all(&script)?;

            // write the contract file
            let contract_path = src.join("Contract.sol");
            fs::write(contract_path, include_str!("../../../assets/ContractTemplate.sol"))?;
            // write the tests
            let contract_path = test.join("Contract.t.sol");
            fs::write(contract_path, include_str!("../../../assets/ContractTemplate.t.sol"))?;
            // write the script
            let contract_path = script.join("Contract.s.sol");
            fs::write(contract_path, include_str!("../../../assets/ContractTemplate.s.sol"))?;

            let dest = root.join(Config::FILE_NAME);
            let mut config = Config::load_with_root(&root);
            if !dest.exists() {
                // write foundry.toml
                fs::write(dest, config.clone().into_basic().to_string_pretty()?)?;
            }

            // sets up git
            if !no_git {
                init_git_repo(&root, no_commit)?;
            }

            if !offline {
                let opts = DependencyInstallOpts { no_git, no_commit, quiet };

                if root.join("lib/forge-std").exists() {
                    println!("\"lib/forge-std\" already exists, skipping install....");
                    install(&mut config, vec![], opts)?;
                } else {
                    Dependency::from_str("https://github.com/foundry-rs/forge-std")
                        .and_then(|dependency| install(&mut config, vec![dependency], opts))?;
                }
            }
            // vscode init
            if vscode {
                init_vscode(&root)?;
            }
        }

        p_println!(!quiet => "    {} forge project.",   Paint::green("Initialized"));
        Ok(())
    }
}

/// Returns `true` if `root` is already in an existing git repository
fn is_git(root: &Path) -> eyre::Result<bool> {
    let is_git = Command::new("git")
        .args(&["rev-parse", "--is-inside-work-tree"])
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait()?;

    Ok(is_git.success())
}

/// Initialises the `root` as git repository if it's not a git repository yet
fn init_git_repo(root: &Path, no_commit: bool) -> eyre::Result<()> {
    if !is_git(root)? {
        let gitignore_path = root.join(".gitignore");
        fs::write(gitignore_path, include_str!("../../../assets/.gitignoreTemplate"))?;

        // git init
        Command::new("git").arg("init").current_dir(&root).exec()?;

        // create github workflow
        let gh = root.join(".github").join("workflows");
        fs::create_dir_all(&gh)?;
        let workflow_path = gh.join("test.yml");
        fs::write(workflow_path, include_str!("../../../assets/workflowTemplate.yml"))?;

        if !no_commit {
            Command::new("git").args(&["add", "."]).current_dir(&root).exec()?;
            Command::new("git")
                .args(&["commit", "-m", "chore: forge init"])
                .current_dir(&root)
                .exec()?;
        }
    }

    Ok(())
}

/// initializes the `.vscode/settings.json` file
fn init_vscode(root: &Path) -> eyre::Result<()> {
    let remappings_file = root.join("remappings.txt");
    if !remappings_file.exists() {
        let mut remappings = relative_remappings(&root.join("lib"), root)
            .into_iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>();
        remappings.sort();
        if !remappings.is_empty() {
            let content = remappings.join("\n");
            fs::write(remappings_file, content)?;
        }
    }

    let vscode_dir = root.join(".vscode");
    let settings_file = vscode_dir.join("settings.json");
    let mut settings = if !vscode_dir.is_dir() {
        fs::create_dir_all(&vscode_dir)?;
        serde_json::json!({})
    } else if settings_file.exists() {
        ethers::solc::utils::read_json_file(&settings_file)?
    } else {
        serde_json::json!({})
    };

    let obj = settings.as_object_mut().expect("Expected settings object");
    // insert [vscode-solidity settings](https://github.com/juanfranblanco/vscode-solidity)
    let src_key = "solidity.packageDefaultDependenciesContractsDirectory";
    if !obj.contains_key(src_key) {
        obj.insert(src_key.to_string(), serde_json::Value::String("src".to_string()));
    }
    let lib_key = "solidity.packageDefaultDependenciesDirectory";
    if !obj.contains_key(lib_key) {
        obj.insert(lib_key.to_string(), serde_json::Value::String("lib".to_string()));
    }

    let content = serde_json::to_string_pretty(&settings)?;
    fs::write(settings_file, content)?;

    Ok(())
}

/// Returns all remappings found in the `lib` path relative to `root`
fn relative_remappings(lib: &Path, root: &Path) -> Vec<Remapping> {
    Remapping::find_many(lib)
        .into_iter()
        .map(|r| r.into_relative(root).to_relative_remapping())
        .map(Into::into)
        .collect()
}
