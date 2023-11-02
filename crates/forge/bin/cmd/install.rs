use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_cli::{
    opts::Dependency,
    p_println, prompt,
    utils::{CommandUtils, Git, LoadConfig},
};
use foundry_common::fs;
use foundry_config::{impl_figment_convert_basic, Config};
use once_cell::sync::Lazy;
use regex::Regex;
use semver::Version;
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
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
    dependencies: Vec<Dependency>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    #[clap(flatten)]
    opts: DependencyInstallOpts,
}

impl_figment_convert_basic!(InstallArgs);

impl InstallArgs {
    pub fn run(self) -> Result<()> {
        let mut config = self.try_load_config_emit_warnings()?;
        self.opts.install(&mut config, self.dependencies)
    }
}

#[derive(Debug, Clone, Default, Copy, Parser)]
pub struct DependencyInstallOpts {
    /// Perform shallow clones instead of deep ones.
    ///
    /// Improves performance and reduces disk usage, but prevents switching branches or tags.
    #[clap(long)]
    pub shallow: bool,

    /// Install without adding the dependency as a submodule.
    #[clap(long)]
    pub no_git: bool,

    /// Do not create a commit.
    #[clap(long)]
    pub no_commit: bool,

    /// Do not print any messages.
    #[clap(short, long)]
    pub quiet: bool,
}

impl DependencyInstallOpts {
    pub fn git(self, config: &Config) -> Git<'_> {
        Git::from_config(config).quiet(self.quiet).shallow(self.shallow)
    }

    /// Installs all missing dependencies.
    ///
    /// See also [`Self::install`].
    ///
    /// Returns true if any dependency was installed.
    pub fn install_missing_dependencies(mut self, config: &mut Config) -> bool {
        let DependencyInstallOpts { quiet, .. } = self;
        let lib = config.install_lib_dir();
        if self.git(config).has_missing_dependencies(Some(lib)).unwrap_or(false) {
            // The extra newline is needed, otherwise the compiler output will overwrite the message
            p_println!(!quiet => "Missing dependencies found. Installing now...\n");
            self.no_commit = true;
            if self.install(config, Vec::new()).is_err() && !quiet {
                eprintln!(
                    "{}",
                    Paint::yellow(
                        "Your project has missing dependencies that could not be installed."
                    )
                )
            }
            true
        } else {
            false
        }
    }

    /// Installs all dependencies
    pub fn install(self, config: &mut Config, dependencies: Vec<Dependency>) -> Result<()> {
        let DependencyInstallOpts { no_git, no_commit, quiet, .. } = self;

        let git = self.git(config);

        let install_lib_dir = config.install_lib_dir();
        let libs = git.root.join(install_lib_dir);

        if dependencies.is_empty() && !self.no_git {
            p_println!(!self.quiet => "Updating dependencies in {}", libs.display());
            // recursively fetch all submodules (without fetching latest)
            git.submodule_update(false, false, false, true, Some(&libs))?;
        }
        fs::create_dir_all(&libs)?;

        let installer = Installer { git, no_commit };
        for dep in dependencies {
            let path = libs.join(dep.name());
            let rel_path = path
                .strip_prefix(git.root)
                .wrap_err("Library directory is not relative to the repository root")?;
            p_println!(!quiet => "Installing {} in {} (url: {:?}, tag: {:?})", dep.name, path.display(), dep.url, dep.tag);

            // this tracks the actual installed tag
            let installed_tag;
            if no_git {
                installed_tag = installer.install_as_folder(&dep, &path)?;
            } else {
                if !no_commit {
                    git.ensure_clean()?;
                }
                installed_tag = installer.install_as_submodule(&dep, &path)?;

                // Pin branch to submodule if branch is used
                if let Some(branch) = &installed_tag {
                    // First, check if this tag has a branch
                    if git.has_branch(branch)? {
                        // always work with relative paths when directly modifying submodules
                        git.cmd()
                            .args(["submodule", "set-branch", "-b", branch])
                            .arg(rel_path)
                            .exec()?;
                    }

                    // update .gitmodules which is at the root of the repo,
                    // not necessarily at the root of the current Foundry project
                    let root = Git::root_of(git.root)?;
                    git.root(&root).add(Some(".gitmodules"))?;
                }

                // commit the installation
                if !no_commit {
                    let mut msg = String::with_capacity(128);
                    msg.push_str("forge install: ");
                    msg.push_str(dep.name());
                    if let Some(tag) = &installed_tag {
                        msg.push_str("\n\n");
                        msg.push_str(tag);
                    }
                    git.commit(&msg)?;
                }
            }

            if !quiet {
                let mut msg = format!("    {} {}", Paint::green("Installed"), dep.name);
                if let Some(tag) = dep.tag.or(installed_tag) {
                    msg.push(' ');
                    msg.push_str(tag.as_str());
                }
                println!("{msg}");
            }
        }

        // update `libs` in config if not included yet
        if !config.libs.iter().any(|p| p == install_lib_dir) {
            config.libs.push(install_lib_dir.to_path_buf());
            config.update_libs()?;
        }
        Ok(())
    }
}

pub fn install_missing_dependencies(config: &mut Config, quiet: bool) -> bool {
    DependencyInstallOpts { quiet, ..Default::default() }.install_missing_dependencies(config)
}

#[derive(Clone, Copy, Debug)]
struct Installer<'a> {
    git: Git<'a>,
    no_commit: bool,
}

impl Installer<'_> {
    /// Installs the dependency as an ordinary folder instead of a submodule
    fn install_as_folder(self, dep: &Dependency, path: &Path) -> Result<Option<String>> {
        let url = dep.require_url()?;
        Git::clone(dep.tag.is_none(), url, Some(&path))?;
        let mut dep = dep.clone();

        if dep.tag.is_none() {
            // try to find latest semver release tag
            dep.tag = self.last_tag(path);
        }

        // checkout the tag if necessary
        self.git_checkout(&dep, path, false)?;

        // remove git artifacts
        fs::remove_dir_all(path.join(".git"))?;

        Ok(dep.tag)
    }

    /// Installs the dependency as new submodule.
    ///
    /// This will add the git submodule to the given dir, initialize it and checkout the tag if
    /// provided or try to find the latest semver, release tag.
    fn install_as_submodule(self, dep: &Dependency, path: &Path) -> Result<Option<String>> {
        // install the dep
        self.git_submodule(dep, path)?;

        let mut dep = dep.clone();
        if dep.tag.is_none() {
            // try to find latest semver release tag
            dep.tag = self.last_tag(path);
        }

        // checkout the tag if necessary
        self.git_checkout(&dep, path, true)?;

        if !self.no_commit {
            self.git.add(Some(path))?;
        }

        Ok(dep.tag)
    }

    fn last_tag(self, path: &Path) -> Option<String> {
        if self.git.shallow {
            None
        } else {
            self.git_semver_tags(path).ok().and_then(|mut tags| tags.pop()).map(|(tag, _)| tag)
        }
    }

    /// Returns all semver git tags sorted in ascending order
    fn git_semver_tags(self, path: &Path) -> Result<Vec<(String, Version)>> {
        let out = self.git.root(path).tag()?;
        let mut tags = Vec::new();
        // tags are commonly prefixed which would make them not semver: v1.2.3 is not a semantic
        // version
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

    /// Install the given dependency as git submodule in `target_dir`.
    fn git_submodule(self, dep: &Dependency, path: &Path) -> Result<()> {
        let url = dep.require_url()?;

        // make path relative to the git root, already checked above
        let path = path.strip_prefix(self.git.root).unwrap();

        trace!(?dep, url, ?path, "installing git submodule");
        self.git.submodule_add(true, url, path)?;

        trace!("initializing submodule recursively");
        self.git.submodule_update(false, false, false, true, Some(path))
    }

    fn git_checkout(self, dep: &Dependency, path: &Path, recurse: bool) -> Result<String> {
        // no need to checkout if there is no tag
        let Some(mut tag) = dep.tag.clone() else { return Ok(String::new()) };

        let mut is_branch = false;
        // only try to match tag if current terminal is a tty
        if std::io::stdout().is_terminal() {
            if tag.is_empty() {
                tag = self.match_tag(&tag, path)?;
            } else if let Some(branch) = self.match_branch(&tag, path)? {
                trace!(?tag, ?branch, "selecting branch for given tag");
                tag = branch;
                is_branch = true;
            }
        }
        let url = dep.url.as_ref().unwrap();

        let res = self.git.root(path).checkout(recurse, &tag);
        if let Err(mut e) = res {
            // remove dependency on failed checkout
            fs::remove_dir_all(path)?;
            if e.to_string().contains("did not match any file(s) known to git") {
                e = eyre::eyre!("Tag: \"{tag}\" not found for repo \"{url}\"!")
            }
            return Err(e)
        }

        if is_branch {
            Ok(tag)
        } else {
            Ok(String::new())
        }
    }

    /// disambiguate tag if it is a version tag
    fn match_tag(self, tag: &str, path: &Path) -> Result<String> {
        // only try to match if it looks like a version tag
        if !DEPENDENCY_VERSION_TAG_REGEX.is_match(tag) {
            return Ok(tag.into())
        }

        // generate candidate list by filtering `git tag` output, valid ones are those "starting
        // with" the user-provided tag (ignoring the starting 'v'), for example, if the user
        // specifies 1.5, then v1.5.2 is a valid candidate, but v3.1.5 is not
        let trimmed_tag = tag.trim_start_matches('v').to_string();
        let output = self.git.root(path).tag()?;
        let mut candidates: Vec<String> = output
            .trim()
            .lines()
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
            let input: String =
                prompt!("Please select a tag (0-{}, default: 1): ", n_candidates - 1)?;
            let s = input.trim();
            // default selection, return first candidate
            let n = if s.is_empty() { Ok(1) } else { s.parse() };
            // match user input, 0 indicates skipping and use original tag
            match n {
                Ok(0) => return Ok(tag.into()),
                Ok(i) if (1..=n_candidates).contains(&i) => {
                    let c = &candidates[i];
                    println!("[{i}] {c} selected");
                    return Ok(c.clone())
                }
                _ => continue,
            }
        }
    }

    fn match_branch(self, tag: &str, path: &Path) -> Result<Option<String>> {
        // fetch remote branches and check for tag
        let output = self.git.root(path).cmd().args(["branch", "-r"]).get_stdout_lossy()?;

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
            Ok(0) => Ok(Some(tag.into())),
            Ok(i) if (1..=n_candidates).contains(&i) => {
                let c = &candidates[i];
                println!("[{i}] {c} selected");
                Ok(Some(c.clone()))
            }
            _ => Ok(None),
        }
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
    use foundry_test_utils::tempfile::tempdir;

    #[test]
    fn get_oz_tags() {
        let tmp = tempdir().unwrap();
        let git = Git::new(tmp.path());
        let installer = Installer { git, no_commit: true };

        git.init().unwrap();

        let dep: Dependency = "openzeppelin/openzeppelin-contracts".parse().unwrap();
        let libs = tmp.path().join("libs");
        fs::create_dir(&libs).unwrap();
        let submodule = libs.join("openzeppelin-contracts");
        installer.git_submodule(&dep, &submodule).unwrap();
        assert!(submodule.exists());

        let tags = installer.git_semver_tags(&submodule).unwrap();
        assert!(!tags.is_empty());
        let v480: Version = "4.8.0".parse().unwrap();
        assert!(tags.iter().any(|(_, v)| v == &v480));
    }
}
