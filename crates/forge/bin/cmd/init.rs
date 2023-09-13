use super::install::DependencyInstallOpts;
use clap::{Parser, ValueHint};
use ethers::solc::remappings::Remapping;
use eyre::Result;
use foundry_cli::{p_println, utils::Git};
use foundry_common::fs;
use foundry_config::Config;
use std::path::{Path, PathBuf};
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

    /// Branch argument that can only be used with template option.
    /// If not specified, the default branch is used.
    #[clap(long, short, requires = "template")]
    branch: Option<String>,

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

    #[clap(flatten)]
    opts: DependencyInstallOpts,
}

impl InitArgs {
    pub fn run(self) -> Result<()> {
        let InitArgs { root, template, branch, opts, offline, force, vscode } = self;
        let DependencyInstallOpts { shallow, no_git, no_commit, quiet } = opts;

        // create the root dir if it does not exist
        if !root.exists() {
            fs::create_dir_all(&root)?;
        }
        let root = dunce::canonicalize(root)?;
        let git = Git::new(&root).quiet(quiet).shallow(shallow);

        // if a template is provided, then this command clones the template repo, removes the .git
        // folder, and initializes a new git repo—-this ensures there is no history from the
        // template and the template is not set as a remote.
        if let Some(template) = template {
            let template = if template.contains("://") {
                template
            } else {
                "https://github.com/".to_string() + &template
            };
            p_println!(!quiet => "Initializing {} from {}...", root.display(), template);

            if let Some(branch) = branch {
                Git::clone_with_branch(shallow, &template, branch, Some(&root))?;
            } else {
                Git::clone(shallow, &template, Some(&root))?;
            }
            // Modify the git history.
            let commit_hash = git.commit_hash(true)?;
            std::fs::remove_dir_all(root.join(".git"))?;

            git.init()?;
            git.add(Some("--all"))?;
            let commit_msg = format!("chore: init from {template} at {commit_hash}");
            git.commit(&commit_msg)?;
        } else {
            // if target is not empty
            if root.read_dir().map_or(false, |mut i| i.next().is_some()) {
                if !force {
                    eyre::bail!(
                        "Cannot run `init` on a non-empty directory.\n\
                        Run with the `--force` flag to initialize regardless."
                    );
                }

                p_println!(!quiet => "Target directory is not empty, but `--force` was specified");
            }

            // ensure git status is clean before generating anything
            if !no_git && !no_commit && !force && git.is_in_repo()? {
                git.ensure_clean()?;
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
            fs::write(contract_path, include_str!("../../assets/CounterTemplate.sol"))?;
            // write the tests
            let contract_path = test.join("Counter.t.sol");
            fs::write(contract_path, include_str!("../../assets/CounterTemplate.t.sol"))?;
            // write the script
            let contract_path = script.join("Counter.s.sol");
            fs::write(contract_path, include_str!("../../assets/CounterTemplate.s.sol"))?;
            // Write the default README file
            let readme_path = root.join("README.md");
            fs::write(readme_path, include_str!("../../assets/README.md"))?;

            // write foundry.toml, if it doesn't exist already
            let dest = root.join(Config::FILE_NAME);
            let mut config = Config::load_with_root(&root);
            if !dest.exists() {
                fs::write(dest, config.clone().into_basic().to_string_pretty()?)?;
            }
            let git = self.opts.git(&config);

            // set up the repo
            if !no_git {
                init_git_repo(git, no_commit)?;
            }

            // install forge-std
            if !offline {
                if root.join("lib/forge-std").exists() {
                    p_println!(!quiet => "\"lib/forge-std\" already exists, skipping install....");
                    self.opts.install(&mut config, vec![])?;
                } else {
                    let dep = "https://github.com/foundry-rs/forge-std".parse()?;
                    self.opts.install(&mut config, vec![dep])?;
                }
            }

            // init vscode settings
            if vscode {
                init_vscode(&root)?;
            }
        }

        p_println!(!quiet => "    {} forge project",   Paint::green("Initialized"));
        Ok(())
    }
}

/// Returns the commit hash of the project if it exists
pub fn get_commit_hash(root: &Path) -> Option<String> {
    Git::new(root).commit_hash(true).ok()
}

/// Initialises `root` as a git repository, if it isn't one already.
///
/// Creates `.gitignore` and `.github/workflows/test.yml`, if they don't exist already.
///
/// Commits everything in `root` if `no_commit` is false.
fn init_git_repo(git: Git<'_>, no_commit: bool) -> Result<()> {
    // git init
    if !git.is_in_repo()? {
        git.init()?;
    }

    // .gitignore
    let gitignore = git.root.join(".gitignore");
    if !gitignore.exists() {
        fs::write(gitignore, include_str!("../../assets/.gitignoreTemplate"))?;
    }

    // github workflow
    let workflow = git.root.join(".github/workflows/test.yml");
    if !workflow.exists() {
        fs::create_dir_all(workflow.parent().unwrap())?;
        fs::write(workflow, include_str!("../../assets/workflowTemplate.yml"))?;
    }

    // commit everything
    if !no_commit {
        git.add(Some("--all"))?;
        git.commit("chore: forge init")?;
    }

    Ok(())
}

/// initializes the `.vscode/settings.json` file
fn init_vscode(root: &Path) -> Result<()> {
    let remappings_file = root.join("remappings.txt");
    if !remappings_file.exists() {
        let mut remappings = Remapping::find_many(root.join("lib"))
            .into_iter()
            .map(|r| r.into_relative(root).to_relative_remapping().to_string())
            .collect::<Vec<_>>();
        if !remappings.is_empty() {
            remappings.sort();
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
