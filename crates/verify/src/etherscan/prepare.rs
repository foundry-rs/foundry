use crate::{bytecode::VerifyBytecodeArgs, types::VerificationType};
use alloy_primitives::Bytes;
use eyre::{OptionExt, Result};
use foundry_block_explorers::contract::Metadata;
use foundry_common::compile::ProjectCompiler;
use foundry_compilers::{
    artifacts::{BytecodeObject, CompactContractBytecode},
    Artifact,
};
use foundry_config::Config;
use semver::Version;
use serde::{Deserialize, Serialize};
use yansi::Paint;

/// Enum to represent the type of bytecode being verified
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum BytecodeType {
    #[serde(rename = "creation")]
    Creation,
    #[serde(rename = "runtime")]
    Runtime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonResult {
    pub bytecode_type: BytecodeType,
    pub matched: bool,
    pub verification_type: VerificationType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub fn build_project(args: &VerifyBytecodeArgs, config: &Config) -> Result<Bytes> {
    let project = config.project()?;
    let compiler = ProjectCompiler::new();

    let output = compiler.compile(&project)?;

    let artifact = output
        .find_contract(&args.contract)
        .ok_or_eyre("Build Error: Contract artifact not found locally")?;

    let local_bytecode =
        artifact.get_bytecode_object().ok_or_eyre("Contract artifact does not have bytecode")?;

    let local_bytecode = match local_bytecode.as_ref() {
        BytecodeObject::Bytecode(bytes) => bytes,
        BytecodeObject::Unlinked(_) => {
            eyre::bail!("Unlinked bytecode is not supported for verification")
        }
    };

    Ok(local_bytecode.to_owned())
}

pub fn build_using_cache(
    args: &VerifyBytecodeArgs,
    etherscan_settings: &Metadata,
    config: &Config,
) -> Option<Bytes> {
    let project = config.project().ok()?;
    let cache = project.read_cache_file().ok()?;
    let cached_artifacts = cache.read_artifacts::<CompactContractBytecode>().ok()?;

    for (key, value) in cached_artifacts {
        let name = args.contract.name.to_owned() + ".sol";
        let version = etherscan_settings.compiler_version.to_owned();
        // Ignores vyper
        if version.starts_with("vyper:") {
            return None;
        }
        // Parse etherscan version string
        let version = version.split('+').next().unwrap_or("").trim_start_matches('v').to_string();

        // Check if `out/directory` name matches the contract name
        if key.ends_with(name.as_str()) {
            let artifacts =
                value.iter().flat_map(|(_, artifacts)| artifacts.iter()).collect::<Vec<_>>();
            let name = name.replace(".sol", ".json");
            for artifact in artifacts {
                // Check if ABI file matches the name
                if !artifact.file.ends_with(&name) {
                    continue;
                }

                // Check if Solidity version matches
                if let Ok(version) = Version::parse(&version) {
                    if !(artifact.version.major == version.major &&
                        artifact.version.minor == version.minor &&
                        artifact.version.patch == version.patch)
                    {
                        continue;
                    }
                }

                return artifact
                    .artifact
                    .bytecode
                    .as_ref()
                    .and_then(|bytes| bytes.bytes().to_owned())
                    .cloned();
            }

            return None
        }
    }

    None
}

pub fn print_result(
    args: &VerifyBytecodeArgs,
    res: (bool, Option<VerificationType>),
    bytecode_type: BytecodeType,
    json_results: &mut Vec<JsonResult>,
    etherscan_config: &Metadata,
    config: &Config,
) {
    if res.0 {
        if !args.json {
            println!(
                "{} with status {}",
                format!("{bytecode_type:?} code matched").green().bold(),
                res.1.unwrap().green().bold()
            );
        } else {
            let json_res = JsonResult {
                bytecode_type,
                matched: true,
                verification_type: res.1.unwrap(),
                message: None,
            };
            json_results.push(json_res);
        }
    } else if !res.0 && !args.json {
        println!(
            "{}",
            format!(
                "{bytecode_type:?} code did not match - this may be due to varying compiler settings"
            )
            .red()
            .bold()
        );
        let mismatches = find_mismatch_in_settings(etherscan_config, config);
        for mismatch in mismatches {
            println!("{}", mismatch.red().bold());
        }
    } else if !res.0 && args.json {
        let json_res = JsonResult {
            bytecode_type,
            matched: false,
            verification_type: res.1.unwrap(),
            message: Some(format!(
                "{bytecode_type:?} code did not match - this may be due to varying compiler settings"
            )),
        };
        json_results.push(json_res);
    }
}

pub fn try_match(
    local_bytecode: &[u8],
    bytecode: &[u8],
    constructor_args: &[u8],
    match_type: &VerificationType,
    is_runtime: bool,
    has_metadata: bool,
) -> Result<(bool, Option<VerificationType>)> {
    // 1. Try full match
    if *match_type == VerificationType::Full && local_bytecode == bytecode {
        Ok((true, Some(VerificationType::Full)))
    } else {
        try_partial_match(local_bytecode, bytecode, constructor_args, is_runtime, has_metadata)
            .map(|matched| (matched, Some(VerificationType::Partial)))
    }
}

fn try_partial_match(
    mut local_bytecode: &[u8],
    mut bytecode: &[u8],
    constructor_args: &[u8],
    is_runtime: bool,
    has_metadata: bool,
) -> Result<bool> {
    // 1. Check length of constructor args
    if constructor_args.is_empty() || is_runtime {
        // Assume metadata is at the end of the bytecode
        return try_extract_and_compare_bytecode(local_bytecode, bytecode, has_metadata)
    }

    // If not runtime, extract constructor args from the end of the bytecode
    bytecode = &bytecode[..bytecode.len() - constructor_args.len()];
    local_bytecode = &local_bytecode[..local_bytecode.len() - constructor_args.len()];

    try_extract_and_compare_bytecode(local_bytecode, bytecode, has_metadata)
}

fn try_extract_and_compare_bytecode(
    mut local_bytecode: &[u8],
    mut bytecode: &[u8],
    has_metadata: bool,
) -> Result<bool> {
    if has_metadata {
        local_bytecode = extract_metadata_hash(local_bytecode)?;
        bytecode = extract_metadata_hash(bytecode)?;
    }

    // Now compare the local code and bytecode
    Ok(local_bytecode == bytecode)
}

/// @dev This assumes that the metadata is at the end of the bytecode
fn extract_metadata_hash(bytecode: &[u8]) -> Result<&[u8]> {
    // Get the last two bytes of the bytecode to find the length of CBOR metadata
    let metadata_len = &bytecode[bytecode.len() - 2..];
    let metadata_len = u16::from_be_bytes([metadata_len[0], metadata_len[1]]);

    // Now discard the metadata from the bytecode
    Ok(&bytecode[..bytecode.len() - 2 - metadata_len as usize])
}

fn find_mismatch_in_settings(
    etherscan_settings: &Metadata,
    local_settings: &Config,
) -> Vec<String> {
    let mut mismatches: Vec<String> = vec![];
    if etherscan_settings.evm_version != local_settings.evm_version.to_string().to_lowercase() {
        let str = format!(
            "EVM version mismatch: local={}, onchain={}",
            local_settings.evm_version, etherscan_settings.evm_version
        );
        mismatches.push(str);
    }
    let local_optimizer: u64 = if local_settings.optimizer { 1 } else { 0 };
    if etherscan_settings.optimization_used != local_optimizer {
        let str = format!(
            "Optimizer mismatch: local={}, onchain={}",
            local_settings.optimizer, etherscan_settings.optimization_used
        );
        mismatches.push(str);
    }
    if etherscan_settings.runs != local_settings.optimizer_runs as u64 {
        let str = format!(
            "Optimizer runs mismatch: local={}, onchain={}",
            local_settings.optimizer_runs, etherscan_settings.runs
        );
        mismatches.push(str);
    }

    mismatches
}
