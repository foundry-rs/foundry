use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, Token},
    prelude::{artifacts::CompactContractBytecode, ProjectPathsConfig},
    types::{Address, I256, U256},
    utils::hex::{FromHex, FromHexError},
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

fn get_env(key: &str, r#type: &str, is_array: bool) -> Result<Bytes, Bytes> {
    let val = env::var(key).map_err::<Bytes, _>(|e| e.to_string().encode().into())?;
    let val = if is_array { val.split(',').collect() } else { vec![val.as_str()] };

    let parse_bool = |v: &str| v.to_lowercase().parse::<bool>();
    let parse_uint_hex = |v: &str| -> Result<U256, FromHexError> {
        let v = Vec::from_hex(v)?;
        Ok(U256::from_little_endian(&v))
    };
    let parse_uint_dec = |v: &str| U256::from_dec_str(v);
    let parse_int = |v: &str| {
        if v.starts_with("0x") {
            I256::from_hex_str(v).map(|v| v.into_raw())
        } else {
            I256::from_dec_str(v).map(|v| v.into_raw())
        }
    };
    let parse_address = |v: &str| Address::from_str(v);
    let parse_bytes = |v: &str| Vec::from_hex(v.strip_prefix("0x").unwrap_or(&v));
    let parse_string = |v: &str| -> Result<String, ()> { Ok(v.to_string()) };

    val.iter()
        .map(|v| match r#type {
            "bool" => parse_bool(v).map(|v| Token::Bool(v)).map_err(|e| e.to_string()),
            "uint" => {
                let token = if v.starts_with("0x") {
                    parse_uint_hex(v).map_err(|e| e.to_string())
                } else {
                    parse_uint_dec(v).map_err(|e| e.to_string())
                };
                token.map(|v| Token::Uint(v))
            }
            "int" => parse_int(v).map(|v| Token::Int(v)).map_err(|e| e.to_string()),
            "address" => parse_address(v).map(|v| Token::Address(v)).map_err(|e| e.to_string()),
            "bytes32" => parse_bytes(v).map(|v| Token::FixedBytes(v)).map_err(|e| e.to_string()),
            "string" => {
                parse_string(v).map(|v| Token::String(v)).map_err(|_| "can't reach".to_string())
            }
            "bytes" => parse_bytes(v).map(|v| Token::Bytes(v)).map_err(|e| e.to_string()),
            _ => Err(format!("{} is not a supported type", r#type)),
        })
        .collect::<Result<Vec<Token>, String>>()
        .map(|tokens| {
            if is_array {
                [Token::Array(tokens)].encode().into()
            } else {
                [tokens[0].clone()].encode().into()
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
        HEVMCalls::EnvBool(inner) => get_env(&inner.0, "bool", false),
        HEVMCalls::EnvUint(inner) => get_env(&inner.0, "uint", false),
        HEVMCalls::EnvInt(inner) => get_env(&inner.0, "int", false),
        HEVMCalls::EnvAddress(inner) => get_env(&inner.0, "address", false),
        HEVMCalls::EnvBytes32(inner) => get_env(&inner.0, "bytes32", false),
        HEVMCalls::EnvString(inner) => get_env(&inner.0, "string", false),
        HEVMCalls::EnvBytes(inner) => get_env(&inner.0, "bytes", false),
        HEVMCalls::EnvBoolArr(inner) => get_env(&inner.0, "bool", true),
        HEVMCalls::EnvUintArr(inner) => get_env(&inner.0, "uint", true),
        HEVMCalls::EnvIntArr(inner) => get_env(&inner.0, "int", true),
        HEVMCalls::EnvAddressArr(inner) => get_env(&inner.0, "address", true),
        HEVMCalls::EnvBytes32Arr(inner) => get_env(&inner.0, "bytes32", true),
        HEVMCalls::EnvStringArr(inner) => get_env(&inner.0, "string", true),
        HEVMCalls::EnvBytesArr(inner) => get_env(&inner.0, "bytes", true),
        _ => return None,
    })
}
