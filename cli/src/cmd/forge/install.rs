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
    /// Target installation directory can be addded via `<alias>=` suffix.
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
        install(root, self.dependencies, self.opts)
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
        let target_dir = if let Some(alias) = &dep.alias { alias } else { &dep.name };
        let DependencyInstallOpts { no_git, no_commit, quiet } = opts;
        p_println!(!quiet => "Installing {} in {:?}, (url: {}, tag: {:?})", dep.name, &libs.join(&target_dir), dep.url, dep.tag);
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
    let target_dir = if let Some(alias) = &dep.alias { alias } else { &dep.name };
    let output = Command::new("git")
        .args(&["clone", &dep.url, target_dir])
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
    } else if !&output.status.success() {
        eyre::bail!("{}", stderr.trim())
    }

    if let Some(ref tag) = dep.tag {
        Command::new("git")
            .args(&["checkout", tag])
            .current_dir(&libs.join(&target_dir))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait()?;
    }

    // rm git artifacts
    std::fs::remove_dir_all(libs.join(&target_dir).join(".git"))?;

    Ok(())
}

/// installs the dependency as new submodule
fn install_as_submodule(dep: &Dependency, libs: &Path, no_commit: bool) -> eyre::Result<()> {
    // install the dep
    let target_dir = if let Some(alias) = &dep.alias { alias } else { &dep.name };
    let output = Command::new("git")
        .args(&["submodule", "add", &dep.url, target_dir])
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
            &target_dir
        )
    } else if stderr.contains("not a git repository") {
        eyre::bail!("\"{}\" is not a git repository", &dep.url)
    } else if stderr.contains("paths are ignored by one of your .gitignore files") {
        let error =
            stderr.lines().filter(|l| !l.starts_with("hint:")).collect::<Vec<&str>>().join("\n");
        eyre::bail!("{}", error)
    } else if !&output.status.success() {
        eyre::bail!("{}", stderr.trim())
    }

    // call update on it
    Command::new("git")
        .args(&["submodule", "update", "--init", "--recursive", target_dir])
        .current_dir(&libs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait()?;

    // checkout the tag if necessary
    let message = if let Some(ref tag) = dep.tag {
        Command::new("git")
            .args(&["checkout", "--recurse-submodules", tag])
            .current_dir(&libs.join(&target_dir))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait()?;

        if !no_commit {
            Command::new("git").args(&["add", &libs.display().to_string()]).spawn()?.wait()?;
        }
        format!("forge install: {}\n\n{}", target_dir, tag)
    } else {
        format!("forge install: {}", target_dir)
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
