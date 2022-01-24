//! Create command

use crate::{cmd::Cmd, opts::forge::Dependency, utils::p_println};
use ansi_term::Colour;
use clap::Parser;
use foundry_config::find_project_root_path;
use std::{
    path::Path,
    process::{Command, Stdio},
};

/// Command to install dependencies
#[derive(Debug, Clone, Parser)]
pub struct InstallArgs {
    #[clap(help = "the submodule name of the library you want to install")]
    dependencies: Vec<Dependency>,
    #[clap(flatten)]
    opts: DependencyInstallOpts,
}

impl Cmd for InstallArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        install(find_project_root_path()?, self.dependencies, self.opts)
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
    let libs = root.join("libs");
    std::fs::create_dir_all(&libs)?;

    for dep in dependencies {
        let DependencyInstallOpts { no_git, no_commit, quiet } = opts;
        let path = libs.join(&dep.name);
        p_println!(!quiet => "Installing {} in {:?}, (url: {}, tag: {:?})", dep.name, path, dep.url, dep.tag);
        if no_git {
            install_as_folder(&dep, &path)?;
        } else {
            install_as_submodule(&dep, root, &path, no_commit)?;
        }

        p_println!(!quiet => "    {} {}",    Colour::Green.paint("Installed"), dep.name);
    }
    Ok(())
}

/// installs the dependency as an ordinary folder instead of a submodule
fn install_as_folder(dep: &Dependency, path: &Path) -> eyre::Result<()> {
    Command::new("git")
        .args(&["clone", &dep.url, &path.display().to_string()])
        .stdout(Stdio::piped())
        .spawn()?
        .wait()?;

    if let Some(ref tag) = dep.tag {
        Command::new("git")
            .args(&["checkout", tag])
            .current_dir(&path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait()?;
    }

    // rm git artifacts
    std::fs::remove_dir_all(path.join(".git"))?;

    Ok(())
}

/// installs the dependency as new submodule
fn install_as_submodule(
    dep: &Dependency,
    root: &Path,
    path: &Path,
    no_commit: bool,
) -> eyre::Result<()> {
    // install the dep
    Command::new("git")
        .args(&["submodule", "add", &dep.url, &path.display().to_string()])
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait()?;
    // call update on it
    Command::new("git")
        .args(&["submodule", "update", "--init", "--recursive", &path.display().to_string()])
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait()?;

    // checkout the tag if necessary
    let message = if let Some(ref tag) = dep.tag {
        Command::new("git")
            .args(&["checkout", "--recurse-submodules", tag])
            .current_dir(&path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait()?;

        if !no_commit {
            Command::new("git").args(&["add", &path.display().to_string()]).spawn()?.wait()?;
        }
        format!("forge install: {}\n\n{}", dep.name, tag)
    } else {
        format!("forge install: {}", dep.name)
    };

    if !no_commit {
        Command::new("git")
            .args(&["commit", "-m", &message])
            .current_dir(&root)
            .stdout(Stdio::piped())
            .spawn()?
            .wait()?;
    }

    Ok(())
}
