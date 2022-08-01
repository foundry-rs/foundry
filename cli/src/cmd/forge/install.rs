//! Install command
use crate::{
    cmd::Cmd,
    opts::forge::Dependency,
    utils::{p_println, CommandUtils},
};
use atty::{self, Stream};
use clap::{Parser, ValueHint};
use foundry_common::fs;
use foundry_config::{find_project_root_path, Config};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    io::{stdin, stdout, Write},
    path::{Path, PathBuf},
    process::Command,
    str,
};
use tracing::trace;
use yansi::Paint;

static DEPENDENCY_VERSION_TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^v?\d+(\.\d+)*$").unwrap());

/// Command to install dependencies
#[derive(Debug, Clone, Parser)]
#[clap(override_usage = "forge install [OPTIONS] [DEPENDENCIES]...
    forge install [OPTIONS] <github username>/<github project>@<tag>...
    forge install [OPTIONS] <alias>=<github username>/<github project>@<tag>...
    forge install [OPTIONS] <https:// git url>...")]
pub struct InstallArgs {
    /// The dependencies to install.
    ///
    /// A dependency can be a raw URL, or the path to a GitHub repository.
    ///
    /// Additionally, a ref can be provided by adding @ to the dependency path.
    ///
    /// A ref can be:
    /// - A branch: master
    /// - A tag: v1.2.3
    /// - A commit: 8e8128
    ///
    /// Target installation directory can be added via `<alias>=` suffix.
    /// The dependency will installed to `lib/<alias>`.
    #[clap(value_name = "DEPENDENCIES")]
    dependencies: Vec<Dependency>,
    #[clap(flatten)]
    opts: DependencyInstallOpts,
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    pub root: Option<PathBuf>,
}

impl Cmd for InstallArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let InstallArgs { root, .. } = self;
        let root = root.unwrap_or_else(|| find_project_root_path().unwrap());
        install(&root, self.dependencies, self.opts)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Parser)]
pub struct DependencyInstallOpts {
    #[clap(help = "Install without adding the dependency as a submodule.", long)]
    pub no_git: bool,
    #[clap(help = "Do not create a commit.", long)]
    pub no_commit: bool,
    #[clap(help = "Do not print any messages.", short, long)]
    pub quiet: bool,
}

/// Installs all dependencies
pub(crate) fn install(
    root: impl AsRef<Path>,
    dependencies: Vec<Dependency>,
    opts: DependencyInstallOpts,
) -> eyre::Result<()> {
    let root = root.as_ref();
    let mut config = Config::load_with_root(root);

    let install_lib_dir = config.install_lib_dir();
    let libs = root.join(&install_lib_dir);

    if dependencies.is_empty() {
        Command::new("git")
            .args(&[
                "submodule",
                "update",
                "--init",
                "--recursive",
                libs.display().to_string().as_str(),
            ])
            .exec()?;
    }
    fs::create_dir_all(&libs)?;

    for dep in dependencies {
        if dep.url.is_none() {
            eyre::bail!("Could not determine URL for dependency \"{}\"!", dep.name);
        }
        let target_dir = if let Some(alias) = &dep.alias { alias } else { &dep.name };
        let DependencyInstallOpts { no_git, no_commit, quiet } = opts;
        p_println!(!quiet => "Installing {} in {:?} (url: {:?}, tag: {:?})", dep.name, &libs.join(&target_dir), dep.url, dep.tag);
        if no_git {
            install_as_folder(&dep, &libs, target_dir)?;
        } else {
            if !no_commit {
                ensure_git_status_clean(root)?;
            }
            let tag = install_as_submodule(&dep, &libs, target_dir, no_commit)?;

            // Pin branch to submodule if branch is used
            if let Some(branch) = tag {
                if !(branch.is_empty()) {
                    Command::new("git")
                        .args(&[
                            "submodule",
                            "set-branch",
                            "-b",
                            &branch,
                            install_lib_dir.join(&target_dir).to_str().unwrap(),
                        ])
                        .exec()?;
                }
            }
        }

        p_println!(!quiet => "    {} {}",    Paint::green("Installed"), dep.name);
    }

    // update `libs` in config if not included yet
    if !config.libs.contains(&install_lib_dir) {
        config.libs.push(install_lib_dir);
        config.update_libs()?;
    }
    Ok(())
}

/// installs the dependency as an ordinary folder instead of a submodule
fn install_as_folder(dep: &Dependency, libs: &Path, target_dir: &str) -> eyre::Result<()> {
    // install the dep
    git_clone(dep, libs, target_dir)?;

    // checkout the tag if necessary
    git_checkout(dep, libs, target_dir, false)?;

    // remove git artifacts
    fs::remove_dir_all(libs.join(&target_dir).join(".git"))?;

    Ok(())
}

/// installs the dependency as new submodule
fn install_as_submodule(
    dep: &Dependency,
    libs: &Path,
    target_dir: &str,
    no_commit: bool,
) -> eyre::Result<Option<String>> {
    // install the dep
    git_submodule(dep, libs, target_dir)?;

    // checkout the tag if necessary
    let tag = if dep.tag.is_none() {
        None
    } else {
        let tag = git_checkout(dep, libs, target_dir, true)?;
        if !no_commit {
            Command::new("git").args(&["add", &libs.display().to_string()]).exec()?;
        }
        Some(tag)
    };

    // commit the added submodule
    if !no_commit {
        let message = if let Some(tag) = &tag {
            format!("forge install: {target_dir}\n\n{tag}")
        } else {
            format!("forge install: {target_dir}")
        };

        Command::new("git").args(&["commit", "-m", &message]).current_dir(&libs).exec()?;
    }

    Ok(tag)
}

pub fn ensure_git_status_clean(root: impl AsRef<Path>) -> eyre::Result<()> {
    if !git_status_clean(root)? {
        eyre::bail!("This command requires clean working and staging areas, including no untracked files. Modify .gitignore and/or add/commit first, or add the --no-commit option.")
    }
    Ok(())
}

// check that there are no modification in git working/staging area
fn git_status_clean(root: impl AsRef<Path>) -> eyre::Result<bool> {
    let stdout =
        Command::new("git").args(&["status", "--short"]).current_dir(root).get_stdout_lossy()?;
    Ok(stdout.is_empty())
}

fn git_clone(dep: &Dependency, libs: &Path, target_dir: &str) -> eyre::Result<()> {
    let url = dep.url.as_ref().unwrap();

    let output = Command::new("git")
        .args(&["clone", "--recursive", url, target_dir])
        .current_dir(&libs)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("remote: Repository not found") {
        eyre::bail!("Repo: \"{}\" not found!", url)
    } else if stderr.contains("already exists and is not an empty directory") {
        eyre::bail!(
            "Destination path \"{}\" already exists and is not an empty directory.",
            &dep.name
        )
    } else if !&output.status.success() {
        eyre::bail!("{}", stderr.trim())
    }

    Ok(())
}

fn git_submodule(dep: &Dependency, libs: &Path, target_dir: &str) -> eyre::Result<()> {
    let url = dep.url.as_ref().ok_or_else(|| eyre::eyre!("No dependency url"))?;
    trace!("installing git submodule {:?} in {} from `{}`", dep, target_dir, url);

    let output = Command::new("git")
        .args(&["submodule", "add", "--force", url, target_dir])
        .current_dir(&libs)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    trace!(?stderr, "`git submodule add`");

    if stderr.contains("remote: Repository not found") {
        eyre::bail!("Repo: \"{}\" not found!", url)
    } else if stderr.contains("already exists in the index") {
        eyre::bail!(
            "\"lib/{}\" already exists in the index, you can update it using forge update.",
            &target_dir
        )
    } else if stderr.contains("not a git repository") {
        eyre::bail!("{stderr}")
    } else if stderr.contains("paths are ignored by one of your .gitignore files") {
        let error =
            stderr.lines().filter(|l| !l.starts_with("hint:")).collect::<Vec<&str>>().join("\n");
        eyre::bail!("{error}")
    } else if !&output.status.success() {
        eyre::bail!("{}", stderr.trim())
    }

    trace!(?dep, "successfully installed");

    let output = Command::new("git")
        .args(&["submodule", "update", "--init", "--recursive", target_dir])
        .current_dir(&libs)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    trace!(?stderr, ?libs, "`git submodule update --init --recursive` {}", target_dir);

    Ok(())
}

fn git_checkout(
    dep: &Dependency,
    libs: &Path,
    target_dir: &str,
    recurse: bool,
) -> eyre::Result<String> {
    // no need to checkout if there is no tag
    if dep.tag.is_none() {
        return Ok(String::new())
    }

    let mut tag = dep.tag.clone().unwrap();
    let mut is_branch = false;
    // only try to match tag if current terminal is a tty
    if atty::is(Stream::Stdout) {
        if tag.is_empty() {
            tag = match_tag(&tag, libs, target_dir)?;
        } else if let Some(branch) = match_branch(&tag, libs, target_dir)? {
            trace!(?tag, ?branch, "selecting branch for given tag");
            tag = branch;
            is_branch = true;
        }
    }
    let url = dep.url.as_ref().unwrap();

    let checkout = |tag: &str| {
        let args = if recurse {
            vec!["checkout", "--recurse-submodules", tag]
        } else {
            vec!["checkout", tag]
        };
        trace!(?tag, ?recurse, "git checkout");
        Command::new("git").args(args).current_dir(&libs.join(&target_dir)).output()
    };

    let output = checkout(&tag)?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    trace!(?stderr, ?tag, "checked out");

    if !&output.status.success() {
        // remove dependency on failed checkout
        fs::remove_dir_all(libs.join(&target_dir))?;

        if stderr.contains(
            format!("error: pathspec '{tag}' did not match any file(s) known to git").as_str(),
        ) {
            eyre::bail!("Tag: \"{}\" not found for repo: \"{}\"!", tag, url)
        } else {
            eyre::bail!("{}", stderr.trim())
        }
    }

    if is_branch {
        Ok(tag)
    } else {
        Ok(String::new())
    }
}

/// disambiguate tag if it is a version tag
fn match_tag(tag: &String, libs: &Path, target_dir: &str) -> eyre::Result<String> {
    // only try to match if it looks like a version tag
    if !DEPENDENCY_VERSION_TAG_REGEX.is_match(tag) {
        return Ok(tag.into())
    }

    // generate candidate list by filtering `git tag` output, valid ones are those "starting with"
    // the user-provided tag (ignoring the starting 'v'), for example, if the user specifies 1.5,
    // then v1.5.2 is a valid candidate, but v3.1.5 is not
    let trimmed_tag = tag.trim_start_matches('v').to_string();
    let output = Command::new("git")
        .args(&["tag"])
        .current_dir(&libs.join(&target_dir))
        .get_stdout_lossy()?;
    let mut candidates: Vec<String> = output
        .trim()
        .split('\n')
        .filter(|x| x.trim_start_matches('v').starts_with(&trimmed_tag))
        .map(|x| x.to_string())
        .rev()
        .collect();

    // no match found, fall back to the user-provided tag
    if candidates.is_empty() {
        return Ok(tag.into())
    }

    // have exact match
    for candidate in candidates.iter() {
        if candidate == tag {
            return Ok(tag.into())
        }
    }

    // only one candidate, ask whether the user wants to accept or not
    if candidates.len() == 1 {
        let matched_tag = candidates[0].clone();
        print!(
            "Found a similar version tag: {}, do you want to use this insead? ([y]/n): ",
            matched_tag
        );
        stdout().flush()?;
        let mut input = String::new();
        stdin().read_line(&mut input)?;
        input = input.trim().to_lowercase();
        return if input.is_empty() || input == "y" || input == "yes" {
            Ok(matched_tag)
        } else {
            // user rejects, fall back to the user-provided tag
            Ok(tag.into())
        }
    }

    // multiple candidates, ask the user to choose one or skip
    candidates.insert(0, String::from("SKIP AND USE ORIGINAL TAG"));
    println!("There are multiple matching tags:");
    for (i, candidate) in candidates.iter().enumerate() {
        println!("[{}] {}", i, candidate);
    }

    let n_candidates = candidates.len();
    loop {
        print!("Please select a tag (0-{}, default: 1): ", n_candidates - 1);
        stdout().flush()?;
        let mut input = String::new();
        stdin().read_line(&mut input)?;
        // default selection, return first candidate
        if input.trim().is_empty() {
            println!("[1] {} selected", candidates[1]);
            return Ok(candidates[1].clone())
        }
        // match user input, 0 indicates skipping and use original tag
        match input.trim().parse::<usize>() {
            Ok(i) if i == 0 => return Ok(tag.into()),
            Ok(i) if (1..=n_candidates).contains(&i) => {
                println!("[{}] {} selected", i, candidates[i]);
                return Ok(candidates[i].clone())
            }
            _ => continue,
        }
    }
}

fn match_branch(tag: &str, libs: &Path, target_dir: &str) -> eyre::Result<Option<String>> {
    // fetch remote branches and check for tag
    let output = Command::new("git")
        .args(&["branch", "-r"])
        .current_dir(&libs.join(&target_dir))
        .get_stdout_lossy()?;

    let mut candidates = output
        .lines()
        .map(|x| x.trim().trim_start_matches("origin/"))
        .filter(|x| x.starts_with(tag))
        .map(str::to_string)
        .rev()
        .collect::<Vec<_>>();

    trace!(?candidates, ?tag, "found branch candidates");

    // no match found, fall back to the user-provided tag
    if candidates.is_empty() {
        return Ok(None)
    }

    // have exact match
    for candidate in candidates.iter() {
        if candidate.as_str() == tag {
            return Ok(Some(tag.to_string()))
        }
    }

    // only one candidate, ask whether the user wants to accept or not
    if candidates.len() == 1 {
        let matched_tag = candidates[0].clone();
        print!("Found a similar branch: {}, do you want to use this instead? ([y]/n)", matched_tag);
        stdout().flush()?;
        let mut input = String::new();
        stdin().read_line(&mut input)?;
        input = input.trim().to_lowercase();
        return if input.is_empty() || input == "y" || input == "yes" {
            Ok(Some(matched_tag))
        } else {
            Ok(None)
        }
    }

    // multiple candidates, ask the user to choose one or skip
    candidates.insert(0, format!("{} (original branch)", tag));
    println!("There are multiple matching branches:");
    for (i, candidate) in candidates.iter().enumerate() {
        println!("[{}] {}", i, candidate);
    }

    let n_candidates = candidates.len();
    print!("Please select a tag (0-{}, default: 1, Press <enter> to cancel): ", n_candidates - 1);
    stdout().flush()?;
    let mut input = String::new();
    stdin().read_line(&mut input)?;
    let input = input.trim();

    // default selection, return first candidate
    if input.is_empty() {
        println!("cancel branch matching");
        return Ok(None)
    }

    // match user input, 0 indicates skipping and use original tag
    match input.parse::<usize>() {
        Ok(i) if i == 0 => Ok(Some(tag.to_string())),
        Ok(i) if (1..=n_candidates).contains(&i) => {
            println!("[{}] {} selected", i, candidates[i]);
            Ok(Some(candidates.remove(i)))
        }
        _ => Ok(None),
    }
}
