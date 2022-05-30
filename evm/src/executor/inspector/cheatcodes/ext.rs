use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, ParamType, Token},
    prelude::{artifacts::CompactContractBytecode, ProjectPathsConfig},
    types::{Address, I256, U256},
    utils::hex::FromHex,
};
use serde::Deserialize;
use std::{env, fs::File, io::Read, path::Path, process::Command, str::FromStr};

fn ffi(args: &[String]) -> Result<Bytes, Bytes> {
    let output = Command::new(&args[0])
        .args(&args[1..])
        .output()
        .map_err(|err| err.to_string().encode())?
        .stdout;
    let output = unsafe { std::str::from_utf8_unchecked(&output) };
    let decoded = hex::decode(&output.trim().strip_prefix("0x").unwrap_or(output))
        .map_err(|err| err.to_string().encode())?;

    Ok(abi::encode(&[Token::Bytes(decoded.to_vec())]).into())
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

    let mut buffer = String::new();
    File::open(path)
        .map_err(|err| err.to_string().encode())?
        .read_to_string(&mut buffer)
        .map_err(|err| err.to_string().encode())?;

    let bytecode = serde_json::from_str::<ArtifactBytecode>(&buffer)
        .map_err(|err| err.to_string().encode())?;

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

pub fn apply(ffi_enabled: bool, call: &HEVMCalls) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Ffi(inner) => {
            if !ffi_enabled {
                Err("FFI disabled: run again with `--ffi` if you want to allow tests to call external scripts.".to_string().encode().into())
            } else {
                ffi(&inner.0)
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
        _ => return None,
    })
}
