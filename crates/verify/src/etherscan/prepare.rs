use crate::{bytecode::VerifyBytecodeArgs, types::VerificationType};
use eyre::{OptionExt, Result};
use foundry_block_explorers::contract::Metadata;
use foundry_common::compile::ProjectCompiler;
use foundry_compilers::artifacts::CompactContractBytecode;
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
    pub match_type: Option<VerificationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub fn match_bytecodes(
    local_bytecode: &[u8],
    bytecode: &[u8],
    constructor_args: &[u8],
    is_runtime: bool,
    has_metadata: bool,
) -> Option<VerificationType> {
    // 1. Try full match
    if local_bytecode == bytecode {
        Some(VerificationType::Full)
    } else {
        is_partial_match(local_bytecode, bytecode, constructor_args, is_runtime, has_metadata)
            .then_some(VerificationType::Partial)
    }
}

pub fn build_project(
    args: &VerifyBytecodeArgs,
    config: &Config,
) -> Result<CompactContractBytecode> {
    let project = config.project()?;
    let compiler = ProjectCompiler::new();

    let mut output = compiler.compile(&project)?;

    let artifact = output
        .remove_contract(&args.contract)
        .ok_or_eyre("Build Error: Contract artifact not found locally")?;

    Ok(artifact.into_contract_bytecode())
}

pub fn build_using_cache(
    args: &VerifyBytecodeArgs,
    etherscan_settings: &Metadata,
    config: &Config,
) -> Result<CompactContractBytecode> {
    let project = config.project()?;
    let cache = project.read_cache_file()?;
    let cached_artifacts = cache.read_artifacts::<CompactContractBytecode>()?;

    for (key, value) in cached_artifacts {
        let name = args.contract.name.to_owned() + ".sol";
        let version = etherscan_settings.compiler_version.to_owned();
        // Ignores vyper
        if version.starts_with("vyper:") {
            eyre::bail!("Vyper contracts are not supported")
        }
        // Parse etherscan version string
        let version = version.split('+').next().unwrap_or("").trim_start_matches('v').to_string();

        // Check if `out/directory` name matches the contract name
        if key.ends_with(name.as_str()) {
            let name = name.replace(".sol", ".json");
            for artifact in value.into_values().flatten() {
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

                return Ok(artifact.artifact)
            }
        }
    }

    eyre::bail!("couldn't find cached artifact for contract {}", args.contract.name)
}

pub fn print_result(
    args: &VerifyBytecodeArgs,
    res: Option<VerificationType>,
    bytecode_type: BytecodeType,
    json_results: &mut Vec<JsonResult>,
    etherscan_config: &Metadata,
    config: &Config,
) {
    if let Some(res) = res {
        if !args.json {
            println!(
                "{} with status {}",
                format!("{bytecode_type:?} code matched").green().bold(),
                res.green().bold()
            );
        } else {
            let json_res = JsonResult { bytecode_type, match_type: Some(res), message: None };
            json_results.push(json_res);
        }
    } else if !args.json {
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
    } else {
        let json_res = JsonResult {
            bytecode_type,
            match_type: res,
            message: Some(format!(
                "{bytecode_type:?} code did not match - this may be due to varying compiler settings"
            )),
        };
        json_results.push(json_res);
    }
}

fn is_partial_match(
    mut local_bytecode: &[u8],
    mut bytecode: &[u8],
    constructor_args: &[u8],
    is_runtime: bool,
    has_metadata: bool,
) -> bool {
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
) -> bool {
    if has_metadata {
        local_bytecode = extract_metadata_hash(local_bytecode);
        bytecode = extract_metadata_hash(bytecode);
    }

    // Now compare the local code and bytecode
    local_bytecode == bytecode
}

/// @dev This assumes that the metadata is at the end of the bytecode
fn extract_metadata_hash(bytecode: &[u8]) -> &[u8] {
    // Get the last two bytes of the bytecode to find the length of CBOR metadata
    let metadata_len = &bytecode[bytecode.len() - 2..];
    let metadata_len = u16::from_be_bytes([metadata_len[0], metadata_len[1]]);

    // Now discard the metadata from the bytecode
    &bytecode[..bytecode.len() - 2 - metadata_len as usize]
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
