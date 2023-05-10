//! init command

use crate::{
    cmd::{
        forge::install::{ensure_git_status_clean, install, DependencyInstallOpts},
        Cmd,
    },
    opts::Dependency,
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

/// CLI arguments for `forge init`.
#[derive(Debug, Clone, Parser)]
pub struct InitArgs {
    /// The root directory of the new project.
    #[clap(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    /// The template to start from.
    #[clap(long, short)]
    template: Option<String>,

    /// Do not create a git repository.
    #[clap(long, conflicts_with = "template")]
    no_git: bool,

    /// Do not create an initial commit.
    #[clap(long, conflicts_with = "template")]
    no_commit: bool,

    /// Do not print any messages.
    #[clap(long, short)]
    quiet: bool,

    /// Do not install dependencies from the network.
    #[clap(long, conflicts_with = "template", visible_alias = "no-deps")]
    offline: bool,

    /// Create the project even if the specified root directory is not empty.
    #[clap(long, conflicts_with = "template")]
    force: bool,

    /// Create a .vscode/settings.json file with Solidity settings, and generate a remappings.txt
    /// file.
    #[clap(long, conflicts_with = "template")]
    vscode: bool,
}

impl Cmd for InitArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let InitArgs { root, template, no_git, no_commit, quiet, offline, force, vscode } = self;

        // create the root dir if it does not exist
        if !root.exists() {
            fs::create_dir_all(&root)?;
        }
        let root = dunce::canonicalize(root)?;

        // if a template is provided, then this command clones the template repo, removes the .git
        // folder, and initializes a new git repo—-this ensures there is no history from the
        // template and the template is not set as a remote.
        if let Some(template) = template {
            let template = if template.starts_with("https://") {
                template
            } else {
                "https://github.com/".to_string() + &template
            };
            p_println!(!quiet => "Initializing {} from {}...", root.display(), template);

            Command::new("git")
                .args(["clone", "--recursive", &template, &root.display().to_string()])
                .exec()?;

            // Navigate to the newly cloned repo.
            let initial_dir = std::env::current_dir()?;
            std::env::set_current_dir(&root)?;

            // Modify the git history.
            let git_output =
                Command::new("git").args(["rev-parse", "--short", "HEAD"]).output()?.stdout;
            let commit_hash = String::from_utf8(git_output)?;
            std::fs::remove_dir_all(".git")?;
            Command::new("git").arg("init").exec()?;
            Command::new("git").args(["add", "--all"]).exec()?;

            let commit_msg = format!("chore: init from {template} at {commit_hash}");
            Command::new("git").args(["commit", "-m", &commit_msg]).exec()?;

            // Navigate back.
            std::env::set_current_dir(initial_dir)?;
        } else {
            // if target is not empty
            if root.read_dir().map(|mut i| i.next().is_some()).unwrap_or(false) {
                if !force {
                    eyre::bail!(
                        "Cannot run `init` on a non-empty directory.\n\
                        Run with the `--force` flag to initialize regardless."
                    );
                }

                p_println!(!quiet => "Target directory is not empty, but `--force` was specified");
            }

            // ensure git status is clean before generating anything
            if !no_git && !no_commit && !force && is_git(&root)? {
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
            let contract_path = src.join("Counter.sol");
            fs::write(contract_path, include_str!("../../../assets/CounterTemplate.sol"))?;
            // write the tests
            let contract_path = test.join("Counter.t.sol");
            fs::write(contract_path, include_str!("../../../assets/CounterTemplate.t.sol"))?;
            // write the script
            let contract_path = script.join("Counter.s.sol");
            fs::write(contract_path, include_str!("../../../assets/CounterTemplate.s.sol"))?;

            // write foundry.toml, if it doesn't exist already
            let dest = root.join(Config::FILE_NAME);
            let mut config = Config::load_with_root(&root);
            if !dest.exists() {
                fs::write(dest, config.clone().into_basic().to_string_pretty()?)?;
            }

            // sets up git
            if !no_git {
                init_git_repo(&root, no_commit)?;
            }

            // install forge-std
            if !offline {
                let opts = DependencyInstallOpts { no_git, no_commit, quiet };

                if root.join("lib/forge-std").exists() {
                    p_println!(!quiet => "\"lib/forge-std\" already exists, skipping install....");
                    install(&mut config, vec![], opts)?;
                } else {
                    Dependency::from_str("https://github.com/foundry-rs/forge-std")
                        .and_then(|dependency| install(&mut config, vec![dependency], opts))?;
                }
            }

            // init vscode settings
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
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()?;

    Ok(is_git.success())
}

/// Returns the commit hash of the project if it exists
pub fn get_commit_hash(root: &Path) -> Option<String> {
    if is_git(root).ok()? {
        let output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .get_stdout_lossy()
            .ok()?;
        Some(output)
    } else {
        None
    }
}

/// Initialises `root` as a git repository, if it isn't one already.
///
/// Creates `.gitignore` and `.github/workflows/test.yml`, if they don't exist already.
///
/// Commits everything in `root` if `no_commit` is false.
fn init_git_repo(root: &Path, no_commit: bool) -> eyre::Result<()> {
    // git init
    if !is_git(root)? {
        Command::new("git").arg("init").current_dir(root).exec()?;
    }

    // .gitignore
    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        fs::write(gitignore, include_str!("../../../assets/.gitignoreTemplate"))?;
    }

    // github workflow
    let gh = root.join(".github").join("workflows");
    if !gh.exists() {
        fs::create_dir_all(&gh)?;
        let workflow_path = gh.join("test.yml");
        fs::write(workflow_path, include_str!("../../../assets/workflowTemplate.yml"))?;
    }

    // commit everything
    if !no_commit {
        Command::new("git").args(["add", "."]).current_dir(root).exec()?;
        Command::new("git").args(["commit", "-m", "chore: forge init"]).current_dir(root).exec()?;
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
