use crate::{bytecode::VerifyBytecodeArgs, types::VerificationType};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{AnyNetworkBlock, BlockId, Transaction};
use clap::ValueEnum;
use eyre::{OptionExt, Result};
use foundry_block_explorers::{
    contract::{ContractCreationData, ContractMetadata, Metadata},
    errors::EtherscanError,
};
use foundry_common::{abi::encode_args, compile::ProjectCompiler, provider::RetryProvider};
use foundry_compilers::artifacts::{BytecodeHash, CompactContractBytecode, EvmVersion};
use foundry_config::Config;
use foundry_evm::{constants::DEFAULT_CREATE2_DEPLOYER, executors::TracingExecutor, opts::EvmOpts};
use reqwest::Url;
use revm_primitives::{
    db::Database,
    env::{EnvWithHandlerCfg, HandlerCfg},
    Bytecode, Env, SpecId,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use yansi::Paint;

/// Enum to represent the type of bytecode being verified
#[derive(Debug, Serialize, Deserialize, Clone, Copy, ValueEnum)]
pub enum BytecodeType {
    #[serde(rename = "creation")]
    Creation,
    #[serde(rename = "runtime")]
    Runtime,
}

impl BytecodeType {
    /// Check if the bytecode type is creation
    pub fn is_creation(&self) -> bool {
        matches!(self, Self::Creation)
    }

    /// Check if the bytecode type is runtime
    pub fn is_runtime(&self) -> bool {
        matches!(self, Self::Runtime)
    }
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
    bytecode_hash: BytecodeHash,
) -> Option<VerificationType> {
    // 1. Try full match
    if local_bytecode == bytecode {
        // If the bytecode_hash = 'none' in Config. Then it's always a partial match according to
        // sourcify definitions. Ref: https://docs.sourcify.dev/docs/full-vs-partial-match/.
        if bytecode_hash == BytecodeHash::None {
            return Some(VerificationType::Partial);
        }

        Some(VerificationType::Full)
    } else {
        is_partial_match(local_bytecode, bytecode, constructor_args, is_runtime)
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
) -> bool {
    // 1. Check length of constructor args
    if constructor_args.is_empty() || is_runtime {
        // Assume metadata is at the end of the bytecode
        return try_extract_and_compare_bytecode(local_bytecode, bytecode)
    }

    // If not runtime, extract constructor args from the end of the bytecode
    bytecode = &bytecode[..bytecode.len() - constructor_args.len()];
    local_bytecode = &local_bytecode[..local_bytecode.len() - constructor_args.len()];

    try_extract_and_compare_bytecode(local_bytecode, bytecode)
}

fn try_extract_and_compare_bytecode(mut local_bytecode: &[u8], mut bytecode: &[u8]) -> bool {
    local_bytecode = extract_metadata_hash(local_bytecode);
    bytecode = extract_metadata_hash(bytecode);

    // Now compare the local code and bytecode
    local_bytecode == bytecode
}

/// @dev This assumes that the metadata is at the end of the bytecode
fn extract_metadata_hash(bytecode: &[u8]) -> &[u8] {
    // Get the last two bytes of the bytecode to find the length of CBOR metadata
    let metadata_len = &bytecode[bytecode.len() - 2..];
    let metadata_len = u16::from_be_bytes([metadata_len[0], metadata_len[1]]);

    if metadata_len as usize <= bytecode.len() {
        if ciborium::from_reader::<ciborium::Value, _>(
            &bytecode[bytecode.len() - 2 - metadata_len as usize..bytecode.len() - 2],
        )
        .is_ok()
        {
            &bytecode[..bytecode.len() - 2 - metadata_len as usize]
        } else {
            bytecode
        }
    } else {
        bytecode
    }
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

pub fn maybe_predeploy_contract(
    creation_data: Result<ContractCreationData, EtherscanError>,
) -> Result<(Option<ContractCreationData>, bool), eyre::ErrReport> {
    let mut maybe_predeploy = false;
    match creation_data {
        Ok(creation_data) => Ok((Some(creation_data), maybe_predeploy)),
        // Ref: https://explorer.mode.network/api?module=contract&action=getcontractcreation&contractaddresses=0xC0d3c0d3c0D3c0d3C0D3c0D3C0d3C0D3C0D30010
        Err(EtherscanError::EmptyResult { status, message })
            if status == "1" && message == "OK" =>
        {
            maybe_predeploy = true;
            Ok((None, maybe_predeploy))
        }
        // Ref: https://api.basescan.org/api?module=contract&action=getcontractcreation&contractaddresses=0xC0d3c0d3c0D3c0d3C0D3c0D3C0d3C0D3C0D30010&apiKey=YourAPIKey
        Err(EtherscanError::Serde { error: _, content }) if content.contains("GENESIS") => {
            maybe_predeploy = true;
            Ok((None, maybe_predeploy))
        }
        Err(e) => eyre::bail!("Error fetching creation data from verifier-url: {:?}", e),
    }
}

pub fn check_and_encode_args(
    artifact: &CompactContractBytecode,
    args: Vec<String>,
) -> Result<Vec<u8>, eyre::ErrReport> {
    if let Some(constructor) = artifact.abi.as_ref().and_then(|abi| abi.constructor()) {
        if constructor.inputs.len() != args.len() {
            eyre::bail!(
                "Mismatch of constructor arguments length. Expected {}, got {}",
                constructor.inputs.len(),
                args.len()
            );
        }
        encode_args(&constructor.inputs, &args).map(|args| DynSolValue::Tuple(args).abi_encode())
    } else {
        Ok(Vec::new())
    }
}

pub fn check_explorer_args(source_code: ContractMetadata) -> Result<Bytes, eyre::ErrReport> {
    if let Some(args) = source_code.items.first() {
        Ok(args.constructor_arguments.clone())
    } else {
        eyre::bail!("No constructor arguments found from block explorer");
    }
}

pub fn check_args_len(
    artifact: &CompactContractBytecode,
    args: &Bytes,
) -> Result<(), eyre::ErrReport> {
    if let Some(constructor) = artifact.abi.as_ref().and_then(|abi| abi.constructor()) {
        if !constructor.inputs.is_empty() && args.len() == 0 {
            eyre::bail!(
                "Contract expects {} constructor argument(s), but none were provided",
                constructor.inputs.len()
            );
        }
    }
    Ok(())
}

pub async fn get_tracing_executor(
    fork_config: &mut Config,
    fork_blk_num: u64,
    evm_version: EvmVersion,
    evm_opts: EvmOpts,
) -> Result<(Env, TracingExecutor)> {
    fork_config.fork_block_number = Some(fork_blk_num);
    fork_config.evm_version = evm_version;

    let (env, fork, _chain, is_alphanet) =
        TracingExecutor::get_fork_material(fork_config, evm_opts).await?;

    let executor = TracingExecutor::new(
        env.clone(),
        fork,
        Some(fork_config.evm_version),
        false,
        false,
        is_alphanet,
    );

    Ok((env, executor))
}

pub fn configure_env_block(env: &mut Env, block: &AnyNetworkBlock) {
    env.block.timestamp = U256::from(block.header.timestamp);
    env.block.coinbase = block.header.miner;
    env.block.difficulty = block.header.difficulty;
    env.block.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
    env.block.basefee = U256::from(block.header.base_fee_per_gas.unwrap_or_default());
    env.block.gas_limit = U256::from(block.header.gas_limit);
}

pub fn deploy_contract(
    executor: &mut TracingExecutor,
    env: &Env,
    spec_id: SpecId,
    transaction: &Transaction,
) -> Result<Address, eyre::ErrReport> {
    let env_with_handler = EnvWithHandlerCfg::new(Box::new(env.clone()), HandlerCfg::new(spec_id));

    if let Some(to) = transaction.to {
        if to != DEFAULT_CREATE2_DEPLOYER {
            eyre::bail!("Transaction `to` address is not the default create2 deployer i.e the tx is not a contract creation tx.");
        }
        let result = executor.transact_with_env(env_with_handler)?;

        trace!(transact_result = ?result.exit_reason);
        if result.result.len() != 20 {
            eyre::bail!(
                "Failed to deploy contract on fork at block: call result is not exactly 20 bytes"
            );
        }

        Ok(Address::from_slice(&result.result))
    } else {
        let deploy_result = executor.deploy_with_env(env_with_handler, None)?;
        trace!(deploy_result = ?deploy_result.raw.exit_reason);
        Ok(deploy_result.address)
    }
}

pub async fn get_runtime_codes(
    executor: &mut TracingExecutor,
    provider: &RetryProvider,
    address: Address,
    fork_address: Address,
    block: Option<u64>,
) -> Result<(Bytecode, Bytes)> {
    let fork_runtime_code = executor
        .backend_mut()
        .basic(fork_address)?
        .ok_or_else(|| {
            eyre::eyre!(
                "Failed to get runtime code for contract deployed on fork at address {}",
                fork_address
            )
        })?
        .code
        .ok_or_else(|| {
            eyre::eyre!(
                "Bytecode does not exist for contract deployed on fork at address {}",
                fork_address
            )
        })?;

    let onchain_runtime_code = if let Some(block) = block {
        provider.get_code_at(address).block_id(BlockId::number(block)).await?
    } else {
        provider.get_code_at(address).await?
    };

    Ok((fork_runtime_code, onchain_runtime_code))
}

/// Returns `true` if the URL only consists of host.
///
/// This is used to check user input url for missing /api path
#[inline]
pub fn is_host_only(url: &Url) -> bool {
    matches!(url.path(), "/" | "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_only() {
        assert!(!is_host_only(&Url::parse("https://blockscout.net/api").unwrap()));
        assert!(is_host_only(&Url::parse("https://blockscout.net/").unwrap()));
        assert!(is_host_only(&Url::parse("https://blockscout.net").unwrap()));
    }
}
