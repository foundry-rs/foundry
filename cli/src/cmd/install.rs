//! Create command
use std::{path::PathBuf, str};

use crate::{cmd::Cmd, opts::forge::Dependency, utils::p_println};
use ansi_term::Colour;
use clap::{Parser, ValueHint};
use foundry_config::find_project_root_path;

use std::{
    path::Path,
    process::{Command, Stdio},
};

/// Command to install dependencies
#[derive(Debug, Clone, Parser)]
pub struct InstallArgs {
    #[clap(
        help = "installs one or more dependencies as git submodules (will install existing dependencies if no arguments are provided)"
    )]
    dependencies: Vec<Dependency>,
    #[clap(flatten)]
    opts: DependencyInstallOpts,
    #[clap(
        help = "the project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
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
        install(root, self.dependencies, self.opts)
    }
}

#[derive(Debug, Clone, Copy, Default, Parser)]
pub struct DependencyInstallOpts {
    #[clap(help = "install without creating a submodule repository", long)]
    pub no_git: bool,
    #[clap(help = "do not create a commit", long)]
    pub no_commit: bool,
    #[clap(help = "do not print messages", short, long)]
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
        let DependencyInstallOpts { no_git, no_commit, quiet } = opts;
        p_println!(!quiet => "Installing {} in {:?}, (url: {}, tag: {:?})", dep.name, &libs.join(&dep.name), dep.url, dep.tag);
        if no_git {
            install_as_folder(&dep, &libs)?;
        } else {
            install_as_submodule(&dep, &libs, no_commit)?;
        }

        p_println!(!quiet => "    {} {}",    Colour::Green.paint("Installed"), dep.name);
    }
    Ok(())
}

/// installs the dependency as an ordinary folder instead of a submodule
fn install_as_folder(dep: &Dependency, libs: &Path) -> eyre::Result<()> {
    let output = Command::new("git")
        .args(&["clone", &dep.url, &dep.name])
        .current_dir(&libs)
        .stdout(Stdio::piped())
        .output()?;

    let stderr = str::from_utf8(&output.stderr).unwrap();

    if stderr.contains("remote: Repository not found") {
        eyre::bail!("Repo: \"{}\" not found!", &dep.url)
    } else if stderr.contains("already exists and is not an empty directory") {
        eyre::bail!(
            "Destination path \"{}\" already exists and is not an empty directory.",
            &dep.name
        )
    }

    if let Some(ref tag) = dep.tag {
        Command::new("git")
            .args(&["checkout", tag])
            .current_dir(&libs.join(&dep.name))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait()?;
    }

    // rm git artifacts
    std::fs::remove_dir_all(libs.join(&dep.name).join(".git"))?;

    Ok(())
}

/// installs the dependency as new submodule
fn install_as_submodule(dep: &Dependency, libs: &Path, no_commit: bool) -> eyre::Result<()> {
    // install the dep
    let output = Command::new("git")
        .args(&["submodule", "add", &dep.url, &dep.name])
        .current_dir(&libs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    if stderr.contains("remote: Repository not found") {
        eyre::bail!("Repo: \"{}\" not found!", &dep.url)
    } else if stderr.contains("already exists in the index") {
        eyre::bail!(
            "\"lib/{}\" already exists in the index, you can update it using forge update.",
            &dep.name
        )
    } else if stderr.contains("not a git repository") {
        eyre::bail!("\"{}\" is not a git repository", &dep.url)
    }

    // call update on it
    Command::new("git")
        .args(&["submodule", "update", "--init", "--recursive", &dep.name])
        .current_dir(&libs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait()?;

    // checkout the tag if necessary
    let message = if let Some(ref tag) = dep.tag {
        Command::new("git")
            .args(&["checkout", "--recurse-submodules", tag])
            .current_dir(&libs.join(&dep.name))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait()?;

        if !no_commit {
            Command::new("git").args(&["add", &libs.display().to_string()]).spawn()?.wait()?;
        }
        format!("forge install: {}\n\n{}", dep.name, tag)
    } else {
        format!("forge install: {}", dep.name)
    };

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
