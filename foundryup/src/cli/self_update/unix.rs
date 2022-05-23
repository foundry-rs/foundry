use std::{path::Path, process::Command};

use eyre::{bail, Context};

use super::{install_bins, shell};
use crate::utils;

/// wipes the entire
pub(crate) fn delete_foundry_home() -> eyre::Result<()> {
    let foundry_home = utils::foundry_home()?;
    utils::remove_dir("foundry_home", &foundry_home)
}

pub(crate) fn do_remove_from_path() -> eyre::Result<()> {
    for sh in shell::get_available_shells() {
        let source_bytes = format!("{}\n", sh.source_string()?).into_bytes();

        // Check more files for cleanup than normally are updated.
        for rc in sh.rcfiles().iter().filter(|rc| rc.is_file()) {
            let file = utils::read_file("rcfile", rc)?;
            let file_bytes = file.into_bytes();
            // FIXME: This is whitespace sensitive where it should not be.
            if let Some(idx) =
                file_bytes.windows(source_bytes.len()).position(|w| w == source_bytes.as_slice())
            {
                // Here we rewrite the file without the offending line.
                let mut new_bytes = file_bytes[..idx].to_vec();
                new_bytes.extend(&file_bytes[idx + source_bytes.len()..]);
                let new_file = String::from_utf8(new_bytes).unwrap();
                utils::write_file("rcfile", rc, &new_file)?;
            }
        }
    }

    Ok(())
}

pub(crate) fn do_add_to_path() -> eyre::Result<()> {
    for sh in shell::get_available_shells() {
        let source_cmd = sh.source_string()?;
        let source_cmd_with_newline = format!("\n{}", &source_cmd);

        for rc in sh.update_rcs() {
            let cmd_to_write = match utils::read_file("rcfile", &rc) {
                Ok(contents) if contents.contains(&source_cmd) => continue,
                Ok(contents) if !contents.ends_with('\n') => &source_cmd_with_newline,
                _ => &source_cmd,
            };

            utils::append_file("rcfile", &rc, cmd_to_write)
                .with_context(|| format!("could not amend shell profile: '{}'", rc.display()))?;
        }
    }

    Ok(())
}

pub(crate) fn do_write_env_files() -> eyre::Result<()> {
    let mut written = vec![];

    for sh in shell::get_available_shells() {
        let script = sh.env_script();
        // Only write each possible script once.
        if !written.contains(&script) {
            script.write()?;
            written.push(script);
        }
    }

    Ok(())
}

/// Tell the upgrader to replace the foundryup bins, then delete
/// itself.
pub(crate) fn run_update(setup_path: &Path) -> eyre::Result<utils::ExitCode> {
    let status =
        Command::new(setup_path).arg("--self-replace").status().context("unable to run updater")?;

    if !status.success() {
        bail!("self-updated failed to replace foundryup executable");
    }

    Ok(utils::ExitCode(0))
}

/// This function is as the final step of a self-upgrade. It replaces
/// `FOUNDRYHOME_HOME`/bin/foundryup with the running exe, and updates the
/// links to it.
pub(crate) fn self_replace() -> eyre::Result<utils::ExitCode> {
    install_bins()?;

    Ok(utils::ExitCode(0))
}
