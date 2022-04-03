use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, Token},
    prelude::{artifacts::CompactContractBytecode, ProjectPathsConfig},
};
use serde::Deserialize;
use std::{fs::File, io::Read, path::Path, process::Command};

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
        out_dir.join(format!("{}/{}.json", file, contract_name))
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
        _ => return None,
    })
}
