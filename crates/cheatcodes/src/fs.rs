//! Implementations of [`Filesystem`](crate::Group::Filesystem) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_json_abi::ContractObject;
use alloy_primitives::U256;
use alloy_sol_types::SolValue;
use foundry_common::{fs, get_artifact_path};
use foundry_config::fs_permissions::FsAccessKind;
use std::{
    collections::hash_map::Entry,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use walkdir::WalkDir;

impl Cheatcode for existsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        Ok(path.exists().abi_encode())
    }
}

impl Cheatcode for fsMetadataCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

        let metadata = path.metadata()?;

        // These fields not available on all platforms; default to 0
        let [modified, accessed, created] =
            [metadata.modified(), metadata.accessed(), metadata.created()].map(|time| {
                time.unwrap_or(UNIX_EPOCH).duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            });

        Ok(FsMetadata {
            isDir: metadata.is_dir(),
            isSymlink: metadata.is_symlink(),
            length: U256::from(metadata.len()),
            readOnly: metadata.permissions().readonly(),
            modified: U256::from(modified),
            accessed: U256::from(accessed),
            created: U256::from(created),
        }
        .abi_encode())
    }
}

impl Cheatcode for isDirCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        Ok(path.is_dir().abi_encode())
    }
}

impl Cheatcode for isFileCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        Ok(path.is_file().abi_encode())
    }
}

impl Cheatcode for projectRootCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        Ok(state.config.root.display().to_string().abi_encode())
    }
}

impl Cheatcode for unixTimeCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        let difference = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| fmt_err!("failed getting Unix timestamp: {e}"))?;
        Ok(difference.as_millis().abi_encode())
    }
}

impl Cheatcode for closeFileCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

        state.context.opened_read_files.remove(&path);

        Ok(Default::default())
    }
}

impl Cheatcode for copyFileCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { from, to } = self;
        let from = state.config.ensure_path_allowed(from, FsAccessKind::Read)?;
        let to = state.config.ensure_path_allowed(to, FsAccessKind::Write)?;
        state.config.ensure_not_foundry_toml(&to)?;

        let n = fs::copy(from, to)?;
        Ok(n.abi_encode())
    }
}

impl Cheatcode for createDirCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, recursive } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
        if *recursive { fs::create_dir_all(path) } else { fs::create_dir(path) }?;
        Ok(Default::default())
    }
}

impl Cheatcode for readDir_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        read_dir(state, path.as_ref(), 1, false)
    }
}

impl Cheatcode for readDir_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, maxDepth } = self;
        read_dir(state, path.as_ref(), *maxDepth, false)
    }
}

impl Cheatcode for readDir_2Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, maxDepth, followLinks } = self;
        read_dir(state, path.as_ref(), *maxDepth, *followLinks)
    }
}

impl Cheatcode for readFileCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        Ok(fs::read_to_string(path)?.abi_encode())
    }
}

impl Cheatcode for readFileBinaryCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        Ok(fs::read(path)?.abi_encode())
    }
}

impl Cheatcode for readLineCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

        // Get reader for previously opened file to continue reading OR initialize new reader
        let reader = match state.context.opened_read_files.entry(path.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(BufReader::new(fs::open(path)?)),
        };

        let mut line: String = String::new();
        reader.read_line(&mut line)?;

        // Remove trailing newline character, preserving others for cases where it may be important
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }

        Ok(line.abi_encode())
    }
}

impl Cheatcode for readLinkCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { linkPath: path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        let target = fs::read_link(path)?;
        Ok(target.display().to_string().abi_encode())
    }
}

impl Cheatcode for removeDirCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, recursive } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
        if *recursive { fs::remove_dir_all(path) } else { fs::remove_dir(path) }?;
        Ok(Default::default())
    }
}

impl Cheatcode for removeFileCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
        state.config.ensure_not_foundry_toml(&path)?;

        // also remove from the set if opened previously
        state.context.opened_read_files.remove(&path);

        if state.fs_commit {
            fs::remove_file(&path)?;
        }

        Ok(Default::default())
    }
}

impl Cheatcode for writeFileCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, data } = self;
        write_file(state, path.as_ref(), data.as_bytes())
    }
}

impl Cheatcode for writeFileBinaryCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, data } = self;
        write_file(state, path.as_ref(), data)
    }
}

impl Cheatcode for writeLineCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { path, data: line } = self;
        let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
        state.config.ensure_not_foundry_toml(&path)?;

        if state.fs_commit {
            let mut file = std::fs::OpenOptions::new().append(true).create(true).open(path)?;

            writeln!(file, "{line}")?;
        }

        Ok(Default::default())
    }
}

impl Cheatcode for getCodeCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { artifactPath: path } = self;
        let object = read_bytecode(state, path)?;
        if let Some(bin) = object.bytecode {
            Ok(bin.abi_encode())
        } else {
            Err(fmt_err!("No bytecode for contract. Is it abstract or unlinked?"))
        }
    }
}

impl Cheatcode for getDeployedCodeCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { artifactPath: path } = self;
        let object = read_bytecode(state, path)?;
        if let Some(bin) = object.deployed_bytecode {
            Ok(bin.abi_encode())
        } else {
            Err(fmt_err!("No deployed bytecode for contract. Is it abstract or unlinked?"))
        }
    }
}

/// Reads the bytecode object(s) from the matching artifact
fn read_bytecode(state: &Cheatcodes, path: &str) -> Result<ContractObject> {
    let path = get_artifact_path(&state.config.paths, path);
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let data = fs::read_to_string(path)?;
    serde_json::from_str::<ContractObject>(&data).map_err(Into::into)
}

impl Cheatcode for ffiCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { commandInput: input } = self;

        let output = ffi(state, input)?;
        // TODO: check exit code?
        if !output.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(target: "cheatcodes", ?input, ?stderr, "non-empty stderr");
        }
        // we already hex-decoded the stdout in `ffi`
        Ok(output.stdout.abi_encode())
    }
}

impl Cheatcode for tryFfiCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { commandInput: input } = self;
        ffi(state, input).map(|res| res.abi_encode())
    }
}

pub(super) fn write_file(state: &Cheatcodes, path: &Path, contents: &[u8]) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
    // write access to foundry.toml is not allowed
    state.config.ensure_not_foundry_toml(&path)?;

    if state.fs_commit {
        fs::write(path, contents)?;
    }

    Ok(Default::default())
}

fn read_dir(state: &Cheatcodes, path: &Path, max_depth: u64, follow_links: bool) -> Result {
    let root = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let paths: Vec<DirEntry> = WalkDir::new(root)
        .min_depth(1)
        .max_depth(max_depth.try_into().unwrap_or(usize::MAX))
        .follow_links(follow_links)
        .contents_first(false)
        .same_file_system(true)
        .sort_by_file_name()
        .into_iter()
        .map(|entry| match entry {
            Ok(entry) => DirEntry {
                errorMessage: String::new(),
                path: entry.path().display().to_string(),
                depth: entry.depth() as u64,
                isDir: entry.file_type().is_dir(),
                isSymlink: entry.path_is_symlink(),
            },
            Err(e) => DirEntry {
                errorMessage: e.to_string(),
                path: e.path().map(|p| p.display().to_string()).unwrap_or_default(),
                depth: e.depth() as u64,
                isDir: false,
                isSymlink: false,
            },
        })
        .collect();
    Ok(paths.abi_encode())
}

fn ffi(state: &Cheatcodes, input: &[String]) -> Result<FfiResult> {
    ensure!(
        state.config.ffi,
        "FFI is disabled; add the `--ffi` flag to allow tests to call external commands"
    );
    ensure!(!input.is_empty() && !input[0].is_empty(), "can't execute empty command");
    let mut cmd = Command::new(&input[0]);
    cmd.args(&input[1..]);

    debug!(target: "cheatcodes", ?cmd, "invoking ffi");

    let output = cmd
        .current_dir(&state.config.root)
        .output()
        .map_err(|err| fmt_err!("failed to execute command {cmd:?}: {err}"))?;

    // The stdout might be encoded on valid hex, or it might just be a string,
    // so we need to determine which it is to avoid improperly encoding later.
    let trimmed_stdout = String::from_utf8(output.stdout)?;
    let trimmed_stdout = trimmed_stdout.trim();
    let encoded_stdout = if let Ok(hex) = hex::decode(trimmed_stdout) {
        hex
    } else {
        trimmed_stdout.as_bytes().to_vec()
    };
    Ok(FfiResult {
        exitCode: output.status.code().unwrap_or(69),
        stdout: encoded_stdout,
        stderr: output.stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CheatsConfig;
    use std::{path::PathBuf, sync::Arc};

    fn cheats() -> Cheatcodes {
        let config = CheatsConfig {
            ffi: true,
            root: PathBuf::from(&env!("CARGO_MANIFEST_DIR")),
            ..Default::default()
        };
        Cheatcodes { config: Arc::new(config), ..Default::default() }
    }

    #[test]
    fn test_ffi_hex() {
        let msg = b"gm";
        let cheats = cheats();
        let args = ["echo".to_string(), hex::encode(msg)];
        let output = ffi(&cheats, &args).unwrap();
        assert_eq!(output.stdout, msg);
    }

    #[test]
    fn test_ffi_string() {
        let msg = "gm";
        let cheats = cheats();
        let args = ["echo".to_string(), msg.to_string()];
        let output = ffi(&cheats, &args).unwrap();
        assert_eq!(output.stdout, msg.as_bytes());
    }

    #[test]
    fn test_artifact_parsing() {
        let s = include_str!("../../evm/test-data/solc-obj.json");
        let artifact: ContractObject = serde_json::from_str(s).unwrap();
        assert!(artifact.bytecode.is_some());

        let artifact: ContractObject = serde_json::from_str(s).unwrap();
        assert!(artifact.deployed_bytecode.is_some());
    }
}
