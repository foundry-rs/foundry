//! Install command
use crate::{
    cmd::{Cmd, LoadConfig},
    opts::Dependency,
    prompt,
    utils::{p_println, CommandUtils},
};
use atty::{self, Stream};
use clap::{Parser, ValueHint};
use ethers::solc::Project;
use foundry_common::fs;
use foundry_config::{impl_figment_convert_basic, Config};
use once_cell::sync::Lazy;
use regex::Regex;
use semver::Version;
use std::{
    path::{Path, PathBuf},
    process::Command,
    str,
};
use tracing::{trace, warn};
use yansi::Paint;

static DEPENDENCY_VERSION_TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^v?\d+(\.\d+)*$").unwrap());

/// CLI arguments for `forge install`.
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

impl_figment_convert_basic!(InstallArgs);

impl Cmd for InstallArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let mut config = self.try_load_config_emit_warnings()?;
        install(&mut config, self.dependencies, self.opts)?;
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

/// Auto installs missing dependencies
///
/// Note: Since the install-process requires `git` this is only executed if an existing installation
/// of `git` could be found
///
/// See also [`install`]
///
/// Returns whether missing dependencies where installed
pub fn install_missing_dependencies(config: &mut Config, project: &Project, quiet: bool) -> bool {
    // try to auto install missing submodules in the default install dir but only if git is
    // installed
    if which::which("git").is_ok() &&
        has_missing_dependencies(project.root(), config.install_lib_dir())
    {
        // The extra newline is needed, otherwise the compiler output will overwrite the
        // message
        p_println!(!quiet => "Missing dependencies found. Installing now...\n");
        let opts = DependencyInstallOpts { quiet, no_commit: true, ..Default::default() };
        if install(config, Vec::new(), opts).is_err() && !quiet {
            eprintln!(
                "{}",
                Paint::yellow("Your project has missing dependencies that could not be installed.")
            )
        }
        return true
    }

    false
}

/// Installs all dependencies
#[tracing::instrument(name = "install dependencies", skip_all, fields(dependencies, opts))]
pub(crate) fn install(
    config: &mut Config,
    dependencies: Vec<Dependency>,
    opts: DependencyInstallOpts,
) -> eyre::Result<()> {
    let root = config.__root.0.clone();

    let install_lib_dir = config.install_lib_dir();
    let libs = root.join(&install_lib_dir);

    if dependencies.is_empty() && !opts.no_git {
        p_println!(!opts.quiet => "Updating dependencies in {:?}", libs);
        let mut cmd = Command::new("git");
        cmd.current_dir(&root).args([
            "submodule",
            "update",
            "--init",
            "--recursive",
            libs.display().to_string().as_str(),
        ]);
        trace!(?cmd, "updating submodules");
        cmd.exec()?;
    }
    fs::create_dir_all(&libs)?;

    for dep in dependencies {
        if dep.url.is_none() {
            eyre::bail!("Could not determine URL for dependency \"{}\"!", dep.name);
        }
        let target_dir = if let Some(alias) = &dep.alias { alias } else { &dep.name };
        let DependencyInstallOpts { no_git, no_commit, quiet } = opts;
        p_println!(!quiet => "Installing {} in {:?} (url: {:?}, tag: {:?})", dep.name, &libs.join(target_dir), dep.url, dep.tag);

        // this tracks the actual installed tag
        let installed_tag;

        if no_git {
            installed_tag = install_as_folder(&dep, &libs, target_dir)?;
        } else {
            if !no_commit {
                ensure_git_status_clean(&root)?;
            }
            installed_tag = install_as_submodule(&root, &dep, &libs, target_dir, no_commit)?;

            // Pin branch to submodule if branch is used
            if let Some(ref branch) = installed_tag {
                if !branch.is_empty() {
                    let libs = libs.strip_prefix(&root).unwrap_or(&libs);
                    let mut cmd = Command::new("git");
                    cmd.current_dir(&root).args([
                        "submodule",
                        "set-branch",
                        "-b",
                        branch.as_str(),
                        libs.join(target_dir).to_str().unwrap(),
                    ]);
                    trace!(?cmd, "submodule set branch");
                    cmd.exec()?;

                    // this changed the .gitmodules files
                    trace!("git add .gitmodules");
                    Command::new("git").current_dir(&root).args(["add", ".gitmodules"]).exec()?;
                }
            }

            // commit the installation
            if !no_commit {
                commit_after_install(&libs, target_dir, installed_tag.as_deref())?;
            }
        }

        // constructs the message `Installed <name> <branch>?`
        let mut msg = format!("    {} {}", Paint::green("Installed"), dep.name);

        if let Some(tag) = dep.tag.or(installed_tag) {
            msg.push(' ');
            msg.push_str(tag.as_str());
        }

        p_println!(!quiet => "{}", msg);
    }

    // update `libs` in config if not included yet
    if !config.libs.contains(&install_lib_dir) {
        config.libs.push(install_lib_dir);
        config.update_libs()?;
    }
    Ok(())
}

/// Checks if any submodules have not been initialized yet.
///
/// `git submodule status <lib dir>` will return a new line per submodule in the repository. If any
/// line starts with `-` then it has not been initialized yet.
pub fn has_missing_dependencies(root: impl AsRef<Path>, lib_dir: impl AsRef<Path>) -> bool {
    Command::new("git")
        .args(["submodule", "status"])
        .arg(lib_dir.as_ref())
        .current_dir(root)
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout).lines().any(|line| line.starts_with('-'))
        })
        .unwrap_or(false)
}

/// Installs the dependency as an ordinary folder instead of a submodule
fn install_as_folder(
    dep: &Dependency,
    libs: &Path,
    target_dir: &str,
) -> eyre::Result<Option<String>> {
    let repo = git_clone(dep, libs, target_dir)?;
    let mut dep = dep.clone();

    if dep.tag.is_none() {
        // try to find latest semver release tag
        dep.tag = git_semver_tags(&repo).ok().and_then(|mut tags| tags.pop().map(|(tag, _)| tag));
    }

    // checkout the tag if necessary
    git_checkout(&dep, libs, target_dir, false)?;

    // remove git artifacts
    fs::remove_dir_all(repo.join(".git"))?;

    Ok(dep.tag.take())
}

/// Installs the dependency as new submodule.
///
/// This will add the git submodule to the given dir, initialize it and checkout the tag if provided
/// or try to find the latest semver, release tag.
fn install_as_submodule(
    root: &Path,
    dep: &Dependency,
    libs: &Path,
    target_dir: &str,
    no_commit: bool,
) -> eyre::Result<Option<String>> {
    // install the dep
    let submodule = git_submodule(dep, libs, target_dir)?;

    let mut dep = dep.clone();
    if dep.tag.is_none() {
        // try to find latest semver release tag
        dep.tag =
            git_semver_tags(&submodule).ok().and_then(|mut tags| tags.pop().map(|(tag, _)| tag));
    }

    // checkout the tag if necessary
    if dep.tag.is_some() {
        git_checkout(&dep, libs, target_dir, true)?;
        if !no_commit {
            trace!("git add {:?}", libs);
            Command::new("git")
                .current_dir(root)
                .args(["add", &libs.display().to_string()])
                .exec()?;
        }
    }

    Ok(dep.tag.take())
}

/// Commits the git submodule install
fn commit_after_install(libs: &Path, target_dir: &str, tag: Option<&str>) -> eyre::Result<()> {
    let message = if let Some(tag) = tag {
        format!("forge install: {target_dir}\n\n{tag}")
    } else {
        format!("forge install: {target_dir}")
    };
    trace!(?libs, ?message, "git commit -m");

    let output = Command::new("git").args(["commit", "-m", &message]).current_dir(libs).output()?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(?stdout, ?stderr, "git commit -m");

        if !stdout.contains("nothing to commit") {
            eyre::bail!("Failed to commit `{message}`:\n{stdout}\n{stderr}");
        }
    }
    Ok(())
}

pub fn ensure_git_status_clean(root: impl AsRef<Path>) -> eyre::Result<()> {
    if !git_status_clean(root)? {
        eyre::bail!(
            "\
The target directory is a part of or on its own an already initialized git repository,
and it requires clean working and staging areas, including no untracked files.

Check the current git repository's status with `git status`.
Then, you can track files with `git add ...` and then commit them with `git commit`,
ignore them in the `.gitignore` file, or run this command again with the `--no-commit` flag.

If none of the previous steps worked, please open an issue at:
https://github.com/foundry-rs/foundry/issues/new/choose"
        )
    }
    Ok(())
}

// check that there are no modification in git working/staging area
fn git_status_clean(root: impl AsRef<Path>) -> eyre::Result<bool> {
    let stdout =
        Command::new("git").args(["status", "--short"]).current_dir(root).get_stdout_lossy()?;
    Ok(stdout.trim().is_empty())
}

/// Executes a git clone
///
/// Returns the directory of the cloned repository
fn git_clone(dep: &Dependency, libs: &Path, target_dir: &str) -> eyre::Result<PathBuf> {
    let url = dep.url.as_ref().unwrap();

    let output = Command::new("git")
        .args(["clone", "--recursive", url, target_dir])
        .current_dir(libs)
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

    Ok(libs.join(target_dir))
}

/// Returns all semver git tags sorted in ascending order
fn git_semver_tags(repo: &Path) -> eyre::Result<Vec<(String, Version)>> {
    trace!(?repo, "`git tag`");
    let output = Command::new("git").arg("tag").current_dir(repo).output()?;
    let mut tags = Vec::new();
    let out = String::from_utf8_lossy(&output.stdout);
    // tags are commonly prefixed which would make them not semver: v1.2.3 is not a semantic version
    let common_prefixes = &["v-", "v", "release-", "release"];
    for tag in out.lines() {
        let mut maybe_semver = tag;
        for &prefix in common_prefixes {
            if let Some(rem) = tag.strip_prefix(prefix) {
                maybe_semver = rem;
                break
            }
        }
        match Version::parse(maybe_semver) {
            Ok(v) => {
                // ignore if additional metadata, like rc, beta, etc...
                if v.build.is_empty() && v.pre.is_empty() {
                    tags.push((tag.to_string(), v));
                }
            }
            Err(err) => {
                warn!(?err, ?maybe_semver, "No semver tag");
            }
        }
    }

    tags.sort_by(|(_, a), (_, b)| a.cmp(b));

    Ok(tags)
}

/// Install the given dependency as git submodule in the `target_dir`
fn git_submodule(dep: &Dependency, libs: &Path, target_dir: &str) -> eyre::Result<PathBuf> {
    let url = dep.url.as_ref().ok_or_else(|| eyre::eyre!("No dependency url"))?;
    trace!("installing git submodule {dep:?} in {target_dir} from `{url}`");

    let output = Command::new("git")
        .args(["submodule", "add", "--force", url, target_dir])
        .current_dir(libs)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    trace!(?stderr, "`git submodule add --force {url} {target_dir}`");

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
    } else if !output.status.success() {
        eyre::bail!("{}", stderr.trim())
    }

    trace!(?dep, "successfully installed");

    let output = Command::new("git")
        .args(["submodule", "update", "--init", "--recursive", target_dir])
        .current_dir(libs)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    trace!(?stderr, ?libs, "`git submodule update --init --recursive` {}", target_dir);

    Ok(libs.join(target_dir))
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

    trace!(?tag, ?recurse, "git checkout");

    let mut args = vec!["checkout", tag.as_str()];
    if recurse {
        args.push("--recurse-submodules");
    }
    let output = Command::new("git").args(args).current_dir(libs.join(target_dir)).output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    trace!(?stderr, ?tag, "checked out");

    if !output.status.success() {
        // remove dependency on failed checkout
        fs::remove_dir_all(libs.join(target_dir))?;

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
    let output =
        Command::new("git").arg("tag").current_dir(&libs.join(target_dir)).get_stdout_lossy()?;
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
        let matched_tag = &candidates[0];
        let input = prompt!(
            "Found a similar version tag: {matched_tag}, do you want to use this instead? [Y/n] "
        )?;
        return if match_yn(input) { Ok(matched_tag.clone()) } else { Ok(tag.into()) }
    }

    // multiple candidates, ask the user to choose one or skip
    candidates.insert(0, String::from("SKIP AND USE ORIGINAL TAG"));
    println!("There are multiple matching tags:");
    for (i, candidate) in candidates.iter().enumerate() {
        println!("[{i}] {candidate}");
    }

    let n_candidates = candidates.len();
    loop {
        let input: String = prompt!("Please select a tag (0-{}, default: 1): ", n_candidates - 1)?;
        let s = input.trim();
        // default selection, return first candidate
        let n = if s.is_empty() { Ok(1) } else { s.parse() };
        // match user input, 0 indicates skipping and use original tag
        match n {
            Ok(i) if i == 0 => return Ok(tag.into()),
            Ok(i) if (1..=n_candidates).contains(&i) => {
                let c = &candidates[i];
                println!("[{i}] {c} selected");
                return Ok(c.clone())
            }
            _ => continue,
        }
    }
}

fn match_branch(tag: &str, libs: &Path, target_dir: &str) -> eyre::Result<Option<String>> {
    // fetch remote branches and check for tag
    let output = Command::new("git")
        .args(["branch", "-r"])
        .current_dir(&libs.join(target_dir))
        .get_stdout_lossy()?;

    let mut candidates = output
        .lines()
        .map(|x| x.trim().trim_start_matches("origin/"))
        .filter(|x| x.starts_with(tag))
        .map(ToString::to_string)
        .rev()
        .collect::<Vec<_>>();

    trace!(?candidates, ?tag, "found branch candidates");

    // no match found, fall back to the user-provided tag
    if candidates.is_empty() {
        return Ok(None)
    }

    // have exact match
    for candidate in candidates.iter() {
        if candidate == tag {
            return Ok(Some(tag.to_string()))
        }
    }

    // only one candidate, ask whether the user wants to accept or not
    if candidates.len() == 1 {
        let matched_tag = &candidates[0];
        let input = prompt!(
            "Found a similar branch: {matched_tag}, do you want to use this instead? [Y/n] "
        )?;
        return if match_yn(input) { Ok(Some(matched_tag.clone())) } else { Ok(None) }
    }

    // multiple candidates, ask the user to choose one or skip
    candidates.insert(0, format!("{tag} (original branch)"));
    println!("There are multiple matching branches:");
    for (i, candidate) in candidates.iter().enumerate() {
        println!("[{i}] {candidate}");
    }

    let n_candidates = candidates.len();
    let input: String = prompt!(
        "Please select a tag (0-{}, default: 1, Press <enter> to cancel): ",
        n_candidates - 1
    )?;
    let input = input.trim();

    // default selection, return None
    if input.is_empty() {
        println!("Canceled branch matching");
        return Ok(None)
    }

    // match user input, 0 indicates skipping and use original tag
    match input.parse::<usize>() {
        Ok(i) if i == 0 => Ok(Some(tag.into())),
        Ok(i) if (1..=n_candidates).contains(&i) => {
            let c = &candidates[i];
            println!("[{i}] {c} selected");
            Ok(Some(c.clone()))
        }
        _ => Ok(None),
    }
}

/// Matches on the result of a prompt for yes/no.
///
/// Defaults to true.
fn match_yn(input: String) -> bool {
    let s = input.trim().to_lowercase();
    matches!(s.as_str(), "" | "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_cli_test_utils::tempfile::tempdir;

    #[test]
    fn get_oz_tags() {
        let tmp = tempdir().unwrap();
        Command::new("git").arg("init").current_dir(tmp.path()).exec().unwrap();
        let dep: Dependency = "openzeppelin/openzeppelin-contracts".parse().unwrap();
        let libs = tmp.path().join("libs");
        fs::create_dir(&libs).unwrap();
        let target = libs.join("openzeppelin-contracts");
        let submodule = git_submodule(&dep, &libs, "openzeppelin-contracts").unwrap();
        assert!(target.exists());
        assert!(submodule.exists());

        let tags = git_semver_tags(&submodule).unwrap();
        assert!(!tags.is_empty());
        let v480: Version = "4.8.0".parse().unwrap();
        assert!(tags.iter().any(|(_, v)| v == &v480));
    }
}
