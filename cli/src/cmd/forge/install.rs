//! Create command
use atty::{self, Stream};
use std::{
    io::{stdin, stdout, Write},
    path::PathBuf,
    str,
};

use crate::{cmd::Cmd, opts::forge::Dependency, utils::p_println};
use clap::{Parser, ValueHint};
use foundry_config::{find_project_root_path, Config};
use once_cell::sync::Lazy;
use regex::Regex;
use yansi::Paint;

use std::{
    path::Path,
    process::{Command, Stdio},
};

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
    dependencies: Vec<Dependency>,
    #[clap(flatten)]
    opts: DependencyInstallOpts,
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub root: Option<PathBuf>,
}

impl Cmd for InstallArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let InstallArgs { root, .. } = self;
        let root = root.unwrap_or_else(|| find_project_root_path().unwrap());
        install(&root, self.dependencies, self.opts)?;
        let mut config = Config::load_with_root(root);
        let lib = PathBuf::from("lib");
        if !config.libs.contains(&lib) {
            config.libs.push(lib);
            config.update_libs()?;
        }
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
    let libs = root.join("lib");

    if dependencies.is_empty() {
        let mut cmd = Command::new("git");
        cmd.args(&[
            "submodule",
            "update",
            "--init",
            "--recursive",
            libs.display().to_string().as_str(),
        ]);
        cmd.spawn()?.wait()?;
    }
    std::fs::create_dir_all(&libs)?;

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
            install_as_submodule(&dep, &libs, target_dir, no_commit)?;
        }

        p_println!(!quiet => "    {} {}",    Paint::green("Installed"), dep.name);
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
    std::fs::remove_dir_all(libs.join(&target_dir).join(".git"))?;

    Ok(())
}

/// installs the dependency as new submodule
fn install_as_submodule(
    dep: &Dependency,
    libs: &Path,
    target_dir: &str,
    no_commit: bool,
) -> eyre::Result<()> {
    // install the dep
    git_submodule(dep, libs, target_dir)?;

    // checkout the tag if necessary
    let message = if dep.tag.is_none() {
        format!("forge install: {target_dir}")
    } else {
        let tag = git_checkout(dep, libs, target_dir, true)?;
        if !no_commit {
            Command::new("git").args(&["add", &libs.display().to_string()]).spawn()?.wait()?;
        }
        format!("forge install: {target_dir}\n\n{tag}")
    };

    // commit the added submodule
    if !no_commit {
        Command::new("git")
            .args(&["commit", "-m", &message])
            .current_dir(&libs)
            .stdout(Stdio::piped())
            .spawn()?
            .wait()?;
    }

    Ok(())
}

fn git_clone(dep: &Dependency, libs: &Path, target_dir: &str) -> eyre::Result<()> {
    let url = dep.url.as_ref().unwrap();

    let output = Command::new("git")
        .args(&["clone", "--recursive", url, target_dir])
        .current_dir(&libs)
        .stdout(Stdio::piped())
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
    let url = dep.url.as_ref().unwrap();

    let output = Command::new("git")
        .args(&["submodule", "add", url, target_dir])
        .current_dir(&libs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
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

    Command::new("git")
        .args(&["submodule", "update", "--init", "--recursive", target_dir])
        .current_dir(&libs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait()?;

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

    let mut tag = dep.tag.as_ref().unwrap().clone();
    // only try to match tag if current terminal is a tty
    if atty::is(Stream::Stdout) {
        tag = match_tag(&tag, libs, target_dir)?
    }
    let url = dep.url.as_ref().unwrap();

    let args = if recurse {
        vec!["checkout", "--recurse-submodules", &tag]
    } else {
        vec!["checkout", &tag]
    };
    let output = Command::new("git")
        .args(args)
        .current_dir(&libs.join(&target_dir))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !&output.status.success() {
        // remove dependency on failed checkout
        std::fs::remove_dir_all(libs.join(&target_dir))?;

        if stderr.contains(
            format!("error: pathspec '{tag}' did not match any file(s) known to git").as_str(),
        ) {
            eyre::bail!("Tag: \"{}\" not found for repo: \"{}\"!", tag, url)
        } else {
            eyre::bail!("{}", stderr.trim())
        }
    }

    Ok(tag)
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
        .stdout(Stdio::piped())
        .output()?;
    let output = String::from_utf8_lossy(&output.stdout);
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
        // default selection, return fist candidate
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
