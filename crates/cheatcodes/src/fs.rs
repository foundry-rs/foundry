//! Implementations of [`Filesystem`](spec::Group::Filesystem) cheatcodes.

use super::string::parse;
use crate::{Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_dyn_abi::DynSolType;
use alloy_json_abi::ContractObject;
use alloy_primitives::{hex, Bytes, U256};
use alloy_sol_types::SolValue;
use dialoguer::{Input, Password};
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use foundry_evm_core::backend::DatabaseExt;
use revm::interpreter::CreateInputs;
use semver::Version;
use std::{
    collections::hash_map::Entry,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
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
        Ok(get_artifact_code(state, path, false)?.abi_encode())
    }
}

impl Cheatcode for getDeployedCodeCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { artifactPath: path } = self;
        Ok(get_artifact_code(state, path, true)?.abi_encode())
    }
}

impl Cheatcode for deployCode_0Call {
    fn apply_full<DB: DatabaseExt, E: CheatcodesExecutor>(
        &self,
        ccx: &mut CheatsCtxt<DB>,
        executor: &mut E,
    ) -> Result {
        let Self { artifactPath: path } = self;
        let bytecode = get_artifact_code(ccx.state, path, false)?;
        let output = executor
            .exec_create(
                CreateInputs {
                    caller: ccx.caller,
                    scheme: revm::primitives::CreateScheme::Create,
                    value: U256::ZERO,
                    init_code: bytecode,
                    gas_limit: ccx.gas_limit,
                },
                ccx,
            )
            .unwrap();

        Ok(output.address.unwrap().abi_encode())
    }
}

impl Cheatcode for deployCode_1Call {
    fn apply_full<DB: DatabaseExt, E: CheatcodesExecutor>(
        &self,
        ccx: &mut CheatsCtxt<DB>,
        executor: &mut E,
    ) -> Result {
        let Self { artifactPath: path, constructorArgs } = self;
        let mut bytecode = get_artifact_code(ccx.state, path, false)?.to_vec();
        bytecode.extend_from_slice(constructorArgs);
        let output = executor
            .exec_create(
                CreateInputs {
                    caller: ccx.caller,
                    scheme: revm::primitives::CreateScheme::Create,
                    value: U256::ZERO,
                    init_code: bytecode.into(),
                    gas_limit: ccx.gas_limit,
                },
                ccx,
            )
            .unwrap();

        Ok(output.address.unwrap().abi_encode())
    }
}

/// Returns the path to the json artifact depending on the input
///
/// Can parse following input formats:
/// - `path/to/artifact.json`
/// - `path/to/contract.sol`
/// - `path/to/contract.sol:ContractName`
/// - `path/to/contract.sol:ContractName:0.8.23`
/// - `path/to/contract.sol:0.8.23`
/// - `ContractName`
/// - `ContractName:0.8.23`
fn get_artifact_code(state: &Cheatcodes, path: &str, deployed: bool) -> Result<Bytes> {
    let path = if path.ends_with(".json") {
        PathBuf::from(path)
    } else {
        let mut parts = path.split(':');

        let mut file = None;
        let mut contract_name = None;
        let mut version = None;

        let path_or_name = parts.next().unwrap();
        if path_or_name.contains('.') {
            file = Some(PathBuf::from(path_or_name));
            if let Some(name_or_version) = parts.next() {
                if name_or_version.contains('.') {
                    version = Some(name_or_version);
                } else {
                    contract_name = Some(name_or_version);
                    version = parts.next();
                }
            }
        } else {
            contract_name = Some(path_or_name);
            version = parts.next();
        }

        let version = if let Some(version) = version {
            Some(Version::parse(version).map_err(|e| fmt_err!("failed parsing version: {e}"))?)
        } else {
            None
        };

        // Use available artifacts list if present
        if let Some(artifacts) = &state.config.available_artifacts {
            let filtered = artifacts
                .iter()
                .filter(|(id, _)| {
                    // name might be in the form of "Counter.0.8.23"
                    let id_name = id.name.split('.').next().unwrap();

                    if let Some(path) = &file {
                        if !id.source.ends_with(path) {
                            return false;
                        }
                    }
                    if let Some(name) = contract_name {
                        if id_name != name {
                            return false;
                        }
                    }
                    if let Some(ref version) = version {
                        if id.version.minor != version.minor ||
                            id.version.major != version.major ||
                            id.version.patch != version.patch
                        {
                            return false;
                        }
                    }
                    true
                })
                .collect::<Vec<_>>();

            let artifact = match &filtered[..] {
                [] => Err(fmt_err!("no matching artifact found")),
                [artifact] => Ok(artifact),
                filtered => {
                    // If we know the current script/test contract solc version, try to filter by it
                    state
                        .config
                        .running_version
                        .as_ref()
                        .and_then(|version| {
                            let filtered = filtered
                                .iter()
                                .filter(|(id, _)| id.version == *version)
                                .collect::<Vec<_>>();
                            (filtered.len() == 1).then(|| filtered[0])
                        })
                        .ok_or_else(|| fmt_err!("multiple matching artifacts found"))
                }
            }?;

            let maybe_bytecode = if deployed {
                artifact.1.deployed_bytecode().cloned()
            } else {
                artifact.1.bytecode().cloned()
            };

            return maybe_bytecode
                .ok_or_else(|| fmt_err!("no bytecode for contract; is it abstract or unlinked?"));
        } else {
            let path_in_artifacts =
                match (file.map(|f| f.to_string_lossy().to_string()), contract_name) {
                    (Some(file), Some(contract_name)) => {
                        PathBuf::from(format!("{file}/{contract_name}.json"))
                    }
                    (None, Some(contract_name)) => {
                        PathBuf::from(format!("{contract_name}.sol/{contract_name}.json"))
                    }
                    (Some(file), None) => {
                        let name = file.replace(".sol", "");
                        PathBuf::from(format!("{file}/{name}.json"))
                    }
                    _ => bail!("invalid artifact path"),
                };

            state.config.paths.artifacts.join(path_in_artifacts)
        }
    };

    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let data = fs::read_to_string(path)?;
    let artifact = serde_json::from_str::<ContractObject>(&data)?;
    let maybe_bytecode = if deployed { artifact.deployed_bytecode } else { artifact.bytecode };
    maybe_bytecode.ok_or_else(|| fmt_err!("no bytecode for contract; is it abstract or unlinked?"))
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

impl Cheatcode for promptCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { promptText: text } = self;
        prompt(state, text, prompt_input).map(|res| res.abi_encode())
    }
}

impl Cheatcode for promptSecretCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { promptText: text } = self;
        prompt(state, text, prompt_password).map(|res| res.abi_encode())
    }
}

impl Cheatcode for promptSecretUintCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { promptText: text } = self;
        parse(&prompt(state, text, prompt_password)?, &DynSolType::Uint(256))
    }
}

impl Cheatcode for promptAddressCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { promptText: text } = self;
        parse(&prompt(state, text, prompt_input)?, &DynSolType::Address)
    }
}

impl Cheatcode for promptUintCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { promptText: text } = self;
        parse(&prompt(state, text, prompt_input)?, &DynSolType::Uint(256))
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
        stdout: encoded_stdout.into(),
        stderr: output.stderr.into(),
    })
}

fn prompt_input(prompt_text: &str) -> Result<String, dialoguer::Error> {
    Input::new().allow_empty(true).with_prompt(prompt_text).interact_text()
}

fn prompt_password(prompt_text: &str) -> Result<String, dialoguer::Error> {
    Password::new().with_prompt(prompt_text).interact()
}

fn prompt(
    state: &Cheatcodes,
    prompt_text: &str,
    input: fn(&str) -> Result<String, dialoguer::Error>,
) -> Result<String> {
    let text_clone = prompt_text.to_string();
    let timeout = state.config.prompt_timeout;
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let _ = tx.send(input(&text_clone));
    });

    match rx.recv_timeout(timeout) {
        Ok(res) => res.map_err(|err| {
            println!();
            err.to_string().into()
        }),
        Err(_) => {
            println!();
            Err("Prompt timed out".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CheatsConfig;
    use std::sync::Arc;

    fn cheats() -> Cheatcodes {
        let config = CheatsConfig {
            ffi: true,
            root: PathBuf::from(&env!("CARGO_MANIFEST_DIR")),
            ..Default::default()
        };
        Cheatcodes::new(Arc::new(config))
    }

    #[test]
    fn test_ffi_hex() {
        let msg = b"gm";
        let cheats = cheats();
        let args = ["echo".to_string(), hex::encode(msg)];
        let output = ffi(&cheats, &args).unwrap();
        assert_eq!(output.stdout, Bytes::from(msg));
    }

    #[test]
    fn test_ffi_string() {
        let msg = "gm";
        let cheats = cheats();
        let args = ["echo".to_string(), msg.to_string()];
        let output = ffi(&cheats, &args).unwrap();
        assert_eq!(output.stdout, Bytes::from(msg.as_bytes()));
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
