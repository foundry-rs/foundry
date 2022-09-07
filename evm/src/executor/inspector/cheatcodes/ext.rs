use crate::{
    abi::HEVMCalls,
    error,
    executor::inspector::{
        cheatcodes::util::{self},
        Cheatcodes,
    },
};
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, ParamType, Token},
    prelude::artifacts::CompactContractBytecode,
    types::*,
};
use foundry_common::fs;

use foundry_config::fs_permissions::FsAccessKind;
use hex::FromHex;
use jsonpath_rust::JsonPathFinder;
use serde::Deserialize;
use serde_json::Value;
use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};
use tracing::{error, trace};

/// Invokes a `Command` with the given args and returns the abi encoded response
///
/// If the output of the command is valid hex, it returns the hex decoded value
fn ffi(state: &Cheatcodes, args: &[String]) -> Result<Bytes, Bytes> {
    if args.is_empty() || args[0].is_empty() {
        return Err(error::encode_error("Can't execute empty command"))
    }
    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    trace!(?args, "invoking ffi");

    let output = cmd
        .current_dir(&state.config.root)
        .output()
        .map_err(|err| error::encode_error(format!("Failed to execute command: {}", err)))?;

    if !output.stderr.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        error!(?err, "stderr");
    }

    let output = String::from_utf8(output.stdout).map_err(|err| {
        error::encode_error(format!("Failed to decode non utf-8 output: {}", err))
    })?;

    let trim_out = output.trim();
    if let Ok(hex_decoded) = hex::decode(trim_out.strip_prefix("0x").unwrap_or(trim_out)) {
        return Ok(abi::encode(&[Token::Bytes(hex_decoded.to_vec())]).into())
    }

    Ok(trim_out.to_string().encode().into())
}

/// An enum which unifies the deserialization of Hardhat-style artifacts with Forge-style artifacts
/// to get their bytecode.
#[derive(Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
enum ArtifactBytecode {
    Hardhat(HardhatArtifact),
    Forge(CompactContractBytecode),
}

impl ArtifactBytecode {
    fn into_inner(self) -> Option<ethers::types::Bytes> {
        match self {
            ArtifactBytecode::Hardhat(inner) => Some(inner.bytecode),
            ArtifactBytecode::Forge(inner) => {
                inner.bytecode.and_then(|bytecode| bytecode.object.into_bytes())
            }
        }
    }
}

/// A thin wrapper around a Hardhat-style artifact that only extracts the bytecode.
#[derive(Deserialize)]
struct HardhatArtifact {
    #[serde(deserialize_with = "ethers::solc::artifacts::deserialize_bytes")]
    bytecode: ethers::types::Bytes,
}

fn get_code(state: &Cheatcodes, path: &str) -> Result<Bytes, Bytes> {
    let path = if path.ends_with(".json") {
        PathBuf::from(path)
    } else {
        let parts: Vec<&str> = path.split(':').collect();
        let file = parts[0];
        let contract_name =
            if parts.len() == 1 { parts[0].replace(".sol", "") } else { parts[1].to_string() };
        state.config.paths.artifacts.join(format!("{file}/{contract_name}.json"))
    };

    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read_to_string(path).map_err(error::encode_error)?;
    let bytecode = serde_json::from_str::<ArtifactBytecode>(&data).map_err(error::encode_error)?;

    if let Some(bin) = bytecode.into_inner() {
        Ok(abi::encode(&[Token::Bytes(bin.to_vec())]).into())
    } else {
        Err("No bytecode for contract. Is it abstract or unlinked?".to_string().encode().into())
    }
}

fn set_env(key: &str, val: &str) -> Result<Bytes, Bytes> {
    // `std::env::set_var` may panic in the following situations
    // ref: https://doc.rust-lang.org/std/env/fn.set_var.html
    if key.is_empty() {
        Err("Environment variable key can't be empty".to_string().encode().into())
    } else if key.contains('=') {
        Err("Environment variable key can't contain equal sign `=`".to_string().encode().into())
    } else if key.contains('\0') {
        Err("Environment variable key can't contain NUL character `\\0`"
            .to_string()
            .encode()
            .into())
    } else if val.contains('\0') {
        Err("Environment variable value can't contain NUL character `\\0`"
            .to_string()
            .encode()
            .into())
    } else {
        env::set_var(key, val);
        Ok(Bytes::new())
    }
}

fn get_env(key: &str, r#type: ParamType, delim: Option<&str>) -> Result<Bytes, Bytes> {
    let msg = format!("Failed to get environment variable `{}` as type `{}`", key, &r#type);
    let val = env::var(key).map_err::<Bytes, _>(|e| format!("{}: {}", msg, e).encode().into())?;
    let val = if let Some(d) = delim {
        val.split(d).map(|v| v.trim().to_string()).collect()
    } else {
        vec![val]
    };
    let is_array: bool = delim.is_some();
    util::value_to_abi(val, r#type, is_array).map_err(|e| format!("{}: {}", msg, e).encode().into())
}

fn project_root(state: &Cheatcodes) -> Result<Bytes, Bytes> {
    let root = state.config.root.display().to_string();

    Ok(abi::encode(&[Token::String(root)]).into())
}

fn read_file(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read_to_string(path).map_err(error::encode_error)?;

    Ok(abi::encode(&[Token::String(data)]).into())
}

fn read_line(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    // Get reader for previously opened file to continue reading OR initialize new reader
    let reader = state
        .context
        .opened_read_files
        .entry(path.clone())
        .or_insert(BufReader::new(fs::open(path).map_err(error::encode_error)?));

    let mut line: String = String::new();
    reader.read_line(&mut line).map_err(error::encode_error)?;

    // Remove trailing newline character, preserving others for cases where it may be important
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    Ok(abi::encode(&[Token::String(line)]).into())
}

/// Writes the content to the file
///
/// This function will create a file if it does not exist, and will entirely replace its contents if
/// it does.
///
/// Caution: writing files is only allowed if the targeted path is allowed, (inside `<root>/` by
/// default)
fn write_file(state: &Cheatcodes, path: impl AsRef<Path>, content: &str) -> Result<Bytes, Bytes> {
    let path = state
        .config
        .ensure_path_allowed(&path, FsAccessKind::Write)
        .map_err(error::encode_error)?;
    // write access to foundry.toml is not allowed
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    if state.fs_commit {
        fs::write(path, content).map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

/// Writes a single line to the file
///
/// This will create a file if it does not exist but append the `line` if it does
fn write_line(state: &Cheatcodes, path: impl AsRef<Path>, line: &str) -> Result<Bytes, Bytes> {
    let path = state
        .config
        .ensure_path_allowed(&path, FsAccessKind::Write)
        .map_err(error::encode_error)?;
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    if state.fs_commit {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(error::encode_error)?;

        writeln!(file, "{line}").map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

fn close_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(&path, FsAccessKind::Read).map_err(error::encode_error)?;

    state.context.opened_read_files.remove(&path);

    Ok(Bytes::new())
}

/// Removes a file from the filesystem.
///
/// Only files inside `<root>/` can be removed, `foundry.toml` excluded.
///
/// This will return an error if the path points to a directory, or the file does not exist
fn remove_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path = state
        .config
        .ensure_path_allowed(&path, FsAccessKind::Write)
        .map_err(error::encode_error)?;
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    // also remove from the set if opened previously
    state.context.opened_read_files.remove(&path);

    if state.fs_commit {
        fs::remove_file(&path).map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

/// Converts a serde_json::Value to an abi::Token
/// The function is designed to run recursively, so that in case of an object
/// it will call itself to convert each of it's value and encode the whole as a
/// Tuple
fn value_to_token(value: &Value) -> Result<Token, Token> {
    if let Some(boolean) = value.as_bool() {
        Ok(Token::Bool(boolean))
    } else if let Some(string) = value.as_str() {
        if let Some(val) = string.strip_prefix("0x") {
            // If it can decoded as an address, it's an address
            if let Ok(addr) = H160::from_str(string) {
                Ok(Token::Address(addr))
            } else if hex::decode(val).is_ok() {
                // if length == 32 bytes, then encode as Bytes32, else Bytes
                Ok(if val.len() == 64 {
                    Token::FixedBytes(Vec::from_hex(val).unwrap())
                } else {
                    Token::Bytes(Vec::from_hex(val).unwrap())
                })
            } else {
                // If incornrect length, pad 0 at the beginning
                let arr = format!("0{}", val);
                Ok(Token::Bytes(Vec::from_hex(arr).unwrap()))
            }
        } else {
            Ok(Token::String(string.to_owned()))
        }
    } else if let Some(number) = value.as_u64() {
        Ok(Token::Uint(number.into()))
    } else if let Some(number) = value.as_i64() {
        Ok(Token::Int(number.into()))
    } else if let Some(array) = value.as_array() {
        Ok(Token::Array(
            array.iter().map(|val| value_to_token(val).unwrap()).collect::<Vec<Token>>(),
        ))
    } else if let Some(object) = value.as_object() {
        let values =
            object.values().map(|val| value_to_token(val).unwrap()).collect::<Vec<Token>>();
        Ok(Token::Tuple(values))
    } else if value.is_null() {
        Ok(Token::FixedBytes(vec![0; 32]))
    } else {
        Err(Token::String("Could not decode field".to_string()))
    }
}
/// Parses a JSON and returns a single value, an array or an entire JSON object encoded as tuple.
/// As the JSON object is parsed serially, with the keys ordered alphabetically, they must be
/// deserialized in the same order. That means that the solidity `struct` should order it's fields
/// alphabetically and not by efficient packing or some other taxonomy.
fn parse_json(_state: &mut Cheatcodes, json: &str, key: &str) -> Result<Bytes, Bytes> {
    let values: Value = JsonPathFinder::from_str(json, key)?.find();
    // values is an array of items. Depending on the JsonPath key, they
    // can be many or a single item. An item can be a single value or
    // an entire JSON object.
    let res = values
        .as_array()
        .ok_or_else(|| error::encode_error("JsonPath did not return an array"))?
        .iter()
        .map(|inner| value_to_token(inner).map_err(error::encode_error))
        .collect::<Result<Vec<Token>, Bytes>>();
    // encode the bytes as the 'bytes' solidity type
    let abi_encoded = abi::encode(&[Token::Bytes(abi::encode(&res?))]);
    Ok(abi_encoded.into())
}

pub fn apply(
    state: &mut Cheatcodes,
    ffi_enabled: bool,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Ffi(inner) => {
            if !ffi_enabled {
                Err("FFI disabled: run again with `--ffi` if you want to allow tests to call external scripts.".to_string().encode().into())
            } else {
                ffi(state, &inner.0)
            }
        }
        HEVMCalls::GetCode(inner) => get_code(state, &inner.0),
        HEVMCalls::SetEnv(inner) => set_env(&inner.0, &inner.1),
        HEVMCalls::EnvBool0(inner) => get_env(&inner.0, ParamType::Bool, None),
        HEVMCalls::EnvUint0(inner) => get_env(&inner.0, ParamType::Uint(256), None),
        HEVMCalls::EnvInt0(inner) => get_env(&inner.0, ParamType::Int(256), None),
        HEVMCalls::EnvAddress0(inner) => get_env(&inner.0, ParamType::Address, None),
        HEVMCalls::EnvBytes320(inner) => get_env(&inner.0, ParamType::FixedBytes(32), None),
        HEVMCalls::EnvString0(inner) => get_env(&inner.0, ParamType::String, None),
        HEVMCalls::EnvBytes0(inner) => get_env(&inner.0, ParamType::Bytes, None),
        HEVMCalls::EnvBool1(inner) => get_env(&inner.0, ParamType::Bool, Some(&inner.1)),
        HEVMCalls::EnvUint1(inner) => get_env(&inner.0, ParamType::Uint(256), Some(&inner.1)),
        HEVMCalls::EnvInt1(inner) => get_env(&inner.0, ParamType::Int(256), Some(&inner.1)),
        HEVMCalls::EnvAddress1(inner) => get_env(&inner.0, ParamType::Address, Some(&inner.1)),
        HEVMCalls::EnvBytes321(inner) => {
            get_env(&inner.0, ParamType::FixedBytes(32), Some(&inner.1))
        }
        HEVMCalls::EnvString1(inner) => get_env(&inner.0, ParamType::String, Some(&inner.1)),
        HEVMCalls::EnvBytes1(inner) => get_env(&inner.0, ParamType::Bytes, Some(&inner.1)),
        HEVMCalls::ProjectRoot(_) => project_root(state),
        HEVMCalls::ReadFile(inner) => read_file(state, &inner.0),
        HEVMCalls::ReadLine(inner) => read_line(state, &inner.0),
        HEVMCalls::WriteFile(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteLine(inner) => write_line(state, &inner.0, &inner.1),
        HEVMCalls::CloseFile(inner) => close_file(state, &inner.0),
        HEVMCalls::RemoveFile(inner) => remove_file(state, &inner.0),
        // If no key argument is passed, return the whole JSON object.
        // "$" is the JSONPath key for the root of the object
        HEVMCalls::ParseJson0(inner) => parse_json(state, &inner.0, "$"),
        HEVMCalls::ParseJson1(inner) => parse_json(state, &inner.0, &inner.1),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::inspector::CheatsConfig;
    use ethers::core::abi::AbiDecode;
    use std::sync::Arc;

    fn cheats() -> Cheatcodes {
        let config =
            CheatsConfig { root: PathBuf::from(&env!("CARGO_MANIFEST_DIR")), ..Default::default() };
        Cheatcodes { config: Arc::new(config), ..Default::default() }
    }

    #[test]
    fn test_ffi_hex() {
        let msg = "gm";
        let cheats = cheats();
        let args = ["echo".to_string(), hex::encode(msg)];
        let output = ffi(&cheats, &args).unwrap();

        let output = String::decode(&output).unwrap();
        assert_eq!(output, msg);
    }

    #[test]
    fn test_ffi_string() {
        let msg = "gm";
        let cheats = cheats();

        let args = ["echo".to_string(), msg.to_string()];
        let output = ffi(&cheats, &args).unwrap();

        let output = String::decode(&output).unwrap();
        assert_eq!(output, msg);
    }
}
