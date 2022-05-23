//! Paths and Unix shells
//!
//! MacOS, Linux, FreeBSD, and many other OS model their design on Unix,
//! so handling them is relatively consistent. But only relatively.
//! POSIX postdates Unix by 20 years, and each "Unix-like" shell develops
//! unique quirks over time.
//!
//!
//! Windowing Managers, Desktop Environments, GUI Terminals, and PATHs
//!
//! Duplicating paths in PATH can cause performance issues when the OS searches
//! the same place multiple times. Traditionally, Unix configurations have
//! resolved this by setting up PATHs in the shell's login profile.
//!
//! This has its own issues. Login profiles are only intended to run once, but
//! changing the PATH is common enough that people may run it twice. Desktop
//! environments often choose to NOT start login shells in GUI terminals. Thus,
//! a trend has emerged to place PATH updates in other run-commands (rc) files,
//! leaving Rustup with few assumptions to build on for fulfilling its promise
//! to set up PATH appropriately.
//!
//! Rustup addresses this by:
//! 1) using a shell script that updates PATH if the path is not in PATH
//! 2) sourcing this script (`. /path/to/script`) in any appropriate rc file

use super::utils;
use crate::process::get_process;
use eyre::{bail, Result};
use std::{borrow::Cow, path::PathBuf};

pub(crate) type Shell = Box<dyn UnixShell>;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ShellScript {
    content: &'static str,
    name: &'static str,
}

impl ShellScript {
    pub(crate) fn write(&self) -> Result<()> {
        let home = utils::foundry_home()?;
        let foundry_bin = format!("{}/bin", foundry_str()?);
        let env_name = home.join(self.name);
        let env_file = self.content.replace("{foundry_bin}", &foundry_bin);
        utils::write_file(self.name, &env_name, &env_file)?;
        Ok(())
    }
}

pub(crate) fn foundry_str() -> Result<Cow<'static, str>> {
    let path = utils::foundry_home()?;

    let default_foundry_home =
        get_process().home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".foundry");
    Ok(if default_foundry_home == path {
        "$HOME/.foundry".into()
    } else {
        match path.to_str() {
            Some(p) => p.to_owned().into(),
            None => bail!("Non-Unicode path!"),
        }
    })
}

// Cross-platform non-POSIX shells
fn enumerate_shells() -> Vec<Shell> {
    vec![Box::new(Posix), Box::new(Bash), Box::new(Zsh)]
}

pub(crate) fn get_available_shells() -> impl Iterator<Item = Shell> {
    enumerate_shells().into_iter().filter(|sh| sh.does_exist())
}

pub(crate) trait UnixShell {
    // Detects if a shell "exists". Users have multiple shells, so an "eager"
    // heuristic should be used, assuming shells exist if any traces do.
    fn does_exist(&self) -> bool;

    // Gives all rcfiles of a given shell that Rustup is concerned with.
    // Used primarily in checking rcfiles for cleanup.
    fn rcfiles(&self) -> Vec<PathBuf>;

    // Gives rcs that should be written to.
    fn update_rcs(&self) -> Vec<PathBuf>;

    // Writes the relevant env file.
    fn env_script(&self) -> ShellScript {
        ShellScript { name: "env", content: include_str!("env.sh") }
    }

    fn source_string(&self) -> Result<String> {
        Ok(format!(r#". "{}/env""#, foundry_str()?))
    }
}

struct Posix;
impl UnixShell for Posix {
    fn does_exist(&self) -> bool {
        true
    }

    fn rcfiles(&self) -> Vec<PathBuf> {
        match utils::home_dir() {
            Some(dir) => vec![dir.join(".profile")],
            _ => vec![],
        }
    }

    fn update_rcs(&self) -> Vec<PathBuf> {
        // Write to .profile even if it doesn't exist. It's the only rc in the
        // POSIX spec so it should always be set up.
        self.rcfiles()
    }
}

struct Bash;

impl UnixShell for Bash {
    fn does_exist(&self) -> bool {
        !self.update_rcs().is_empty()
    }

    fn rcfiles(&self) -> Vec<PathBuf> {
        // Bash also may read .profile, however Rustup already includes handling
        // .profile as part of POSIX and always does setup for POSIX shells.
        [".bash_profile", ".bash_login", ".bashrc"]
            .iter()
            .filter_map(|rc| utils::home_dir().map(|dir| dir.join(rc)))
            .collect()
    }

    fn update_rcs(&self) -> Vec<PathBuf> {
        self.rcfiles().into_iter().filter(|rc| rc.is_file()).collect()
    }
}

struct Zsh;

impl Zsh {
    fn zdotdir() -> Result<PathBuf> {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        if matches!(get_process().var("SHELL"), Ok(sh) if sh.contains("zsh")) {
            match get_process().var("ZDOTDIR") {
                Ok(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
                _ => bail!("Zsh setup failed."),
            }
        } else {
            match std::process::Command::new("zsh").args(&["-c", "'echo $ZDOTDIR'"]).output() {
                Ok(io) if !io.stdout.is_empty() => Ok(PathBuf::from(OsStr::from_bytes(&io.stdout))),
                _ => bail!("Zsh setup failed."),
            }
        }
    }
}

impl UnixShell for Zsh {
    fn does_exist(&self) -> bool {
        // zsh has to either be the shell or be callable for zsh setup.
        matches!(get_process().var("SHELL"), Ok(sh) if sh.contains("zsh")) ||
            matches!(utils::find_cmd(&["zsh"]), Some(_))
    }

    fn rcfiles(&self) -> Vec<PathBuf> {
        [Zsh::zdotdir().ok(), utils::home_dir()]
            .iter()
            .filter_map(|dir| dir.as_ref().map(|p| p.join(".zshenv")))
            .collect()
    }

    fn update_rcs(&self) -> Vec<PathBuf> {
        // zsh can change $ZDOTDIR both _before_ AND _during_ reading .zshenv,
        // so we: write to $ZDOTDIR/.zshenv if-exists ($ZDOTDIR changes before)
        // OR write to $HOME/.zshenv if it exists (change-during)
        // if neither exist, we create it ourselves, but using the same logic,
        // because we must still respond to whether $ZDOTDIR is set or unset.
        // In any case we only write once.
        self.rcfiles()
            .into_iter()
            .filter(|env| env.is_file())
            .chain(self.rcfiles().into_iter())
            .take(1)
            .collect()
    }
}

pub(crate) fn legacy_paths() -> impl Iterator<Item = PathBuf> {
    let zprofiles =
        Zsh::zdotdir().into_iter().chain(utils::home_dir()).map(|d| d.join(".zprofile"));
    let profiles =
        [".bash_profile", ".profile"].iter().filter_map(|rc| utils::home_dir().map(|d| d.join(rc)));

    profiles.chain(zprofiles)
}
