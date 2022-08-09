use crate::{
    abi::HEVMCalls,
    executor::inspector::{cheatcodes::util, Cheatcodes},
};
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, ParamType, Token},
    prelude::{artifacts::CompactContractBytecode, ProjectPathsConfig},
    types::{Address, I256, U256},
    utils::hex::FromHex,
};
use foundry_common::fs;
use serde::Deserialize;
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
        return Err(util::encode_error("Can't execute empty command"))
    }
    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    trace!(?args, "invoking ffi");

    let output = cmd
        .current_dir(&state.config.root)
        .output()
        .map_err(|err| util::encode_error(format!("Failed to execute command: {}", err)))?;

    if !output.stderr.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        error!(?err, "stderr");
    }

    let output = String::from_utf8(output.stdout)
        .map_err(|err| util::encode_error(format!("Failed to decode non utf-8 output: {}", err)))?;

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

fn get_code(path: &str) -> Result<Bytes, Bytes> {
    let path = if path.ends_with(".json") {
        Path::new(&path).to_path_buf()
    } else {
        let parts: Vec<&str> = path.split(':').collect();
        let file = parts[0];
        let contract_name =
            if parts.len() == 1 { parts[0].replace(".sol", "") } else { parts[1].to_string() };
        let out_dir = ProjectPathsConfig::find_artifacts_dir(Path::new("./"));
        out_dir.join(format!("{file}/{contract_name}.json"))
    };

    let data = fs::read_to_string(path).map_err(util::encode_error)?;
    let bytecode = serde_json::from_str::<ArtifactBytecode>(&data).map_err(util::encode_error)?;

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
    let val = env::var(key).map_err::<Bytes, _>(|e| e.to_string().encode().into())?;
    let val = if let Some(d) = delim {
        val.split(d).map(|v| v.trim()).collect()
    } else {
        vec![val.as_str()]
    };

    let parse_bool = |v: &str| v.to_lowercase().parse::<bool>();
    let parse_uint = |v: &str| {
        if v.starts_with("0x") {
            let v = Vec::from_hex(v.strip_prefix("0x").unwrap()).map_err(|e| e.to_string())?;
            Ok(U256::from_little_endian(&v))
        } else {
            U256::from_dec_str(v).map_err(|e| e.to_string())
        }
    };
    let parse_int = |v: &str| {
        // hex string may start with "0x", "+0x", or "-0x"
        if v.starts_with("0x") || v.starts_with("+0x") || v.starts_with("-0x") {
            I256::from_hex_str(&v.replacen("0x", "", 1)).map(|v| v.into_raw())
        } else {
            I256::from_dec_str(v).map(|v| v.into_raw())
        }
    };
    let parse_address = |v: &str| Address::from_str(v);
    let parse_string = |v: &str| -> Result<String, ()> { Ok(v.to_string()) };
    let parse_bytes = |v: &str| Vec::from_hex(v.strip_prefix("0x").unwrap_or(v));

    val.iter()
        .map(|v| match r#type {
            ParamType::Bool => parse_bool(v).map(Token::Bool).map_err(|e| e.to_string()),
            ParamType::Uint(256) => parse_uint(v).map(Token::Uint),
            ParamType::Int(256) => parse_int(v).map(Token::Int).map_err(|e| e.to_string()),
            ParamType::Address => parse_address(v).map(Token::Address).map_err(|e| e.to_string()),
            ParamType::FixedBytes(32) => {
                parse_bytes(v).map(Token::FixedBytes).map_err(|e| e.to_string())
            }
            ParamType::String => parse_string(v).map(Token::String).map_err(|_| "".to_string()),
            ParamType::Bytes => parse_bytes(v).map(Token::Bytes).map_err(|e| e.to_string()),
            _ => Err(format!("{} is not a supported type", r#type)),
        })
        .collect::<Result<Vec<Token>, String>>()
        .map(|mut tokens| {
            if delim.is_none() {
                abi::encode(&[tokens.remove(0)]).into()
            } else {
                abi::encode(&[Token::Array(tokens)]).into()
            }
        })
        .map_err(|e| e.into())
}

fn full_path(state: &Cheatcodes, path: impl AsRef<Path>) -> PathBuf {
    state.config.root.join(path)
}

fn read_file(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path = full_path(state, &path);
    state.config.ensure_path_allowed(&path).map_err(util::encode_error)?;

    let data = fs::read_to_string(path).map_err(util::encode_error)?;

    Ok(abi::encode(&[Token::String(data)]).into())
}

fn read_line(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path = full_path(state, &path);
    state.config.ensure_path_allowed(&path).map_err(util::encode_error)?;

    // Get reader for previously opened file to continue reading OR initialize new reader
    let reader = state
        .context
        .opened_read_files
        .entry(path.clone())
        .or_insert(BufReader::new(fs::open(path).map_err(util::encode_error)?));

    let mut line: String = String::new();
    reader.read_line(&mut line).map_err(util::encode_error)?;

    // Remove trailing newline character, preserving others for cases where it may be important
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    Ok(abi::encode(&[Token::String(line)]).into())
}

fn write_file(state: &Cheatcodes, path: impl AsRef<Path>, data: &str) -> Result<Bytes, Bytes> {
    let path = full_path(state, &path);
    state.config.ensure_path_allowed(&path).map_err(util::encode_error)?;

    fs::write(path, data).map_err(util::encode_error)?;

    Ok(Bytes::new())
}

fn write_line(state: &Cheatcodes, path: impl AsRef<Path>, line: &str) -> Result<Bytes, Bytes> {
    let path = full_path(state, &path);
    state.config.ensure_path_allowed(&path).map_err(util::encode_error)?;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(util::encode_error)?;

    writeln!(file, "{line}").map_err(util::encode_error)?;

    Ok(Bytes::new())
}

fn close_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path = full_path(state, &path);
    state.config.ensure_path_allowed(&path).map_err(util::encode_error)?;

    state.context.opened_read_files.remove(&path);

    Ok(Bytes::new())
}

fn remove_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path = full_path(state, &path);
    state.config.ensure_path_allowed(&path).map_err(util::encode_error)?;

    close_file(state, &path)?;
    fs::remove_file(&path).map_err(util::encode_error)?;

    Ok(Bytes::new())
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
        HEVMCalls::GetCode(inner) => get_code(&inner.0),
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
        HEVMCalls::ReadFile(inner) => read_file(state, &inner.0),
        HEVMCalls::ReadLine(inner) => read_line(state, &inner.0),
        HEVMCalls::WriteFile(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteLine(inner) => write_line(state, &inner.0, &inner.1),
        HEVMCalls::CloseFile(inner) => close_file(state, &inner.0),
        HEVMCalls::RemoveFile(inner) => remove_file(state, &inner.0),
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
