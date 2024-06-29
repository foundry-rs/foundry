use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use clap::{Parser, ValueHint};
use eyre::{OptionExt, Result};
use foundry_block_explorers::{contract::Metadata, Client};
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, read_constructor_args_file, LoadConfig},
};
use foundry_common::{compile::ProjectCompiler, provider::ProviderBuilder};
use foundry_compilers::{
    artifacts::{BytecodeHash, BytecodeObject, CompactContractBytecode, EvmVersion},
    info::ContractInfo,
    Artifact,
};
use foundry_config::{figment, filter::SkipBuildFilter, impl_figment_convert, Chain, Config};
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER, executors::TracingExecutor, utils::configure_tx_env,
};
use revm_primitives::{db::Database, EnvWithHandlerCfg, HandlerCfg, SpecId};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, str::FromStr};
use yansi::Paint;

impl_figment_convert!(VerifyBytecodeArgs);

/// CLI arguments for `forge verify-bytecode`.
#[derive(Clone, Debug, Parser)]
pub struct VerifyBytecodeArgs {
    /// The address of the contract to verify.
    pub address: Address,

    /// The contract identifier in the form `<path>:<contractname>`.
    pub contract: ContractInfo,

    /// The block at which the bytecode should be verified.
    #[clap(long, value_name = "BLOCK")]
    pub block: Option<BlockId>,

    /// The constructor args to generate the creation code.
    #[clap(
        long,
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
        visible_alias = "encoded-constructor-args"
    )]
    pub constructor_args: Option<String>,

    /// The path to a file containing the constructor arguments.
    #[clap(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub constructor_args_path: Option<PathBuf>,

    /// The rpc url to use for verification.
    #[clap(short = 'r', long, value_name = "RPC_URL", env = "ETH_RPC_URL")]
    pub rpc_url: Option<String>,

    /// Verfication Type: `full` or `partial`.
    /// Ref: <https://docs.sourcify.dev/docs/full-vs-partial-match/>
    #[clap(long, default_value = "full", value_name = "TYPE")]
    pub verification_type: VerificationType,

    #[clap(flatten)]
    pub etherscan_opts: EtherscanOpts,

    /// Skip building files whose names contain the given filter.
    ///
    /// `test` and `script` are aliases for `.t.sol` and `.s.sol`.
    #[arg(long, num_args(1..))]
    pub skip: Option<Vec<SkipBuildFilter>>,

    /// The path to the project's root directory.
    pub root: Option<PathBuf>,

    /// Suppress logs and emit json results to stdout
    #[clap(long, default_value = "false")]
    pub json: bool,
}

impl figment::Provider for VerifyBytecodeArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Bytecode Provider")
    }

    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        let mut dict = figment::value::Dict::new();
        if let Some(block) = &self.block {
            dict.insert("block".into(), figment::value::Value::serialize(block)?);
        }
        if let Some(rpc_url) = &self.rpc_url {
            dict.insert("eth_rpc_url".into(), rpc_url.to_string().into());
        }
        dict.insert("verification_type".into(), self.verification_type.to_string().into());

        Ok(figment::value::Map::from([(Config::selected_profile(), dict)]))
    }
}

impl VerifyBytecodeArgs {
    /// Run the `verify-bytecode` command to verify the bytecode onchain against the locally built
    /// bytecode.
    pub async fn run(mut self) -> Result<()> {
        // Setup
        let config = self.load_config_emit_warnings();
        let provider = ProviderBuilder::new(&config.get_rpc_url_or_localhost_http()?).build()?;

        let code = provider.get_code_at(self.address).await?;
        if code.is_empty() {
            eyre::bail!("No bytecode found at address {}", self.address);
        }

        if !self.json {
            println!(
                "Verifying bytecode for contract {} at address {}",
                self.contract.name.clone().green(),
                self.address.green()
            );
        }

        // If chain is not set, we try to get it from the RPC
        // If RPC is not set, the default chain is used
        let chain = if config.get_rpc_url().is_some() {
            let chain_id = provider.get_chain_id().await?;
            Chain::from(chain_id)
        } else {
            config.chain.unwrap_or_default()
        };

        // Set Etherscan options
        self.etherscan_opts.chain = Some(chain);
        self.etherscan_opts.key =
            config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);

        // If etherscan key is not set, we can't proceed with etherscan verification
        let Some(key) = self.etherscan_opts.key.clone() else {
            eyre::bail!("Etherscan API key is required for verification");
        };
        let etherscan = Client::new(chain, key)?;

        // Get the constructor args using `source_code` endpoint
        let source_code = etherscan.contract_source_code(self.address).await?;

        // Check if the contract name matches
        let name = source_code.items.first().map(|item| item.contract_name.to_owned());
        if name.as_ref() != Some(&self.contract.name) {
            eyre::bail!("Contract name mismatch");
        }

        // Get the constructor args from etherscan
        let constructor_args = if let Some(args) = source_code.items.first() {
            args.constructor_arguments.clone()
        } else {
            eyre::bail!("No constructor arguments found for contract at address {}", self.address);
        };

        // Get user provided constructor args
        let provided_constructor_args = if let Some(args) = self.constructor_args.to_owned() {
            args
        } else if let Some(path) = self.constructor_args_path.to_owned() {
            // Read from file
            let res = read_constructor_args_file(path)?;
            // Convert res to Bytes
            res.join("")
        } else {
            constructor_args.to_string()
        };

        // Constructor args mismatch
        if provided_constructor_args != constructor_args.to_string() && !self.json {
            println!(
                "{}",
                "The provided constructor args do not match the constructor args from etherscan. This will result in a mismatch - Using the args from etherscan".red().bold(),
            );
        }

        // Get creation tx hash
        let creation_data = etherscan.contract_creation_data(self.address).await?;

        let mut transaction = provider
            .get_transaction_by_hash(creation_data.transaction_hash)
            .await
            .or_else(|e| eyre::bail!("Couldn't fetch transaction from RPC: {:?}", e))?
            .ok_or_else(|| {
                eyre::eyre!("Transaction not found for hash {}", creation_data.transaction_hash)
            })?;
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash)
            .await
            .or_else(|e| eyre::bail!("Couldn't fetch transaction receipt from RPC: {:?}", e))?;

        let receipt = if let Some(receipt) = receipt {
            receipt
        } else {
            eyre::bail!(
                "Receipt not found for transaction hash {}",
                creation_data.transaction_hash
            );
        };

        // Extract creation code
        let maybe_creation_code =
            if receipt.to.is_none() && receipt.contract_address == Some(self.address) {
                &transaction.input
            } else if receipt.to == Some(DEFAULT_CREATE2_DEPLOYER) {
                &transaction.input[32..]
            } else {
                eyre::bail!(
                    "Could not extract the creation code for contract at address {}",
                    self.address
                );
            };

        // If bytecode_hash is disabled then its always partial verification
        let (verification_type, has_metadata) =
            match (&self.verification_type, config.bytecode_hash) {
                (VerificationType::Full, BytecodeHash::None) => (VerificationType::Partial, false),
                (VerificationType::Partial, BytecodeHash::None) => {
                    (VerificationType::Partial, false)
                }
                (VerificationType::Full, _) => (VerificationType::Full, true),
                (VerificationType::Partial, _) => (VerificationType::Partial, true),
            };

        trace!(?verification_type, has_metadata);
        // Etherscan compilation metadata
        let etherscan_metadata = source_code.items.first().unwrap();

        let local_bytecode =
            if let Some(local_bytecode) = self.build_using_cache(etherscan_metadata, &config) {
                trace!("using cache");
                local_bytecode
            } else {
                self.build_project(&config)?
            };

        // Append constructor args to the local_bytecode
        let mut local_bytecode_vec = local_bytecode.to_vec();
        local_bytecode_vec.extend_from_slice(&constructor_args);

        // Cmp creation code with locally built bytecode and maybe_creation_code
        let (did_match, with_status) = try_match(
            local_bytecode_vec.as_slice(),
            maybe_creation_code,
            &constructor_args,
            &verification_type,
            false,
            has_metadata,
        )?;

        let mut json_results: Vec<JsonResult> = vec![];
        self.print_result(
            (did_match, with_status),
            BytecodeType::Creation,
            &mut json_results,
            etherscan_metadata,
            &config,
        );

        // Get contract creation block
        let simulation_block = match self.block {
            Some(BlockId::Number(BlockNumberOrTag::Number(block))) => block,
            Some(_) => eyre::bail!("Invalid block number"),
            None => {
                let provider = utils::get_provider(&config)?;
                provider
                    .get_transaction_by_hash(creation_data.transaction_hash)
                    .await.or_else(|e| eyre::bail!("Couldn't fetch transaction from RPC: {:?}", e))?.ok_or_else(|| {
                        eyre::eyre!("Transaction not found for hash {}", creation_data.transaction_hash)
                    })?
                    .block_number.ok_or_else(|| {
                        eyre::eyre!("Failed to get block number of the contract creation tx, specify using the --block flag")
                    })?
            }
        };

        // Fork the chain at `simulation_block`

        let (mut fork_config, evm_opts) = config.clone().load_config_and_evm_opts()?;
        fork_config.fork_block_number = Some(simulation_block - 1);
        fork_config.evm_version =
            etherscan_metadata.evm_version()?.unwrap_or(EvmVersion::default());
        let (mut env, fork, _chain) =
            TracingExecutor::get_fork_material(&fork_config, evm_opts).await?;

        let mut executor =
            TracingExecutor::new(env.clone(), fork, Some(fork_config.evm_version), false);
        env.block.number = U256::from(simulation_block);
        let block = provider.get_block(simulation_block.into(), true.into()).await?;

        // Workaround for the NonceTooHigh issue as we're not simulating prior txs of the same
        // block.
        let prev_block_id = BlockId::number(simulation_block - 1);
        let prev_block_nonce = provider
            .get_transaction_count(creation_data.contract_creator)
            .block_id(prev_block_id)
            .await?;
        transaction.nonce = prev_block_nonce;

        if let Some(ref block) = block {
            env.block.timestamp = U256::from(block.header.timestamp);
            env.block.coinbase = block.header.miner;
            env.block.difficulty = block.header.difficulty;
            env.block.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
            env.block.basefee = U256::from(block.header.base_fee_per_gas.unwrap_or_default());
            env.block.gas_limit = U256::from(block.header.gas_limit);
        }

        configure_tx_env(&mut env, &transaction);

        let env_with_handler =
            EnvWithHandlerCfg::new(Box::new(env.clone()), HandlerCfg::new(SpecId::LATEST));

        let contract_address = if let Some(to) = transaction.to {
            if to != DEFAULT_CREATE2_DEPLOYER {
                eyre::bail!("Transaction `to` address is not the default create2 deployer i.e the tx is not a contract creation tx.");
            }
            let result = executor.transact_with_env(env_with_handler.clone())?;

            if result.result.len() != 20 {
                eyre::bail!("Failed to deploy contract on fork at block {simulation_block}: call result is not exactly 20 bytes");
            }

            Address::from_slice(&result.result)
        } else {
            let deploy_result = executor.deploy_with_env(env_with_handler, None)?;
            deploy_result.address
        };

        // State commited using deploy_with_env, now get the runtime bytecode from the db.
        let fork_runtime_code = executor
            .backend_mut()
            .basic(contract_address)?
            .ok_or_else(|| {
                eyre::eyre!(
                    "Failed to get runtime code for contract deployed on fork at address {}",
                    contract_address
                )
            })?
            .code
            .ok_or_else(|| {
                eyre::eyre!(
                    "Bytecode does not exist for contract deployed on fork at address {}",
                    contract_address
                )
            })?;

        let onchain_runtime_code =
            provider.get_code_at(self.address).block_id(BlockId::number(simulation_block)).await?;

        // Compare the runtime bytecode with the locally built bytecode
        let (did_match, with_status) = try_match(
            fork_runtime_code.bytecode(),
            &onchain_runtime_code,
            &constructor_args,
            &verification_type,
            true,
            has_metadata,
        )?;

        self.print_result(
            (did_match, with_status),
            BytecodeType::Runtime,
            &mut json_results,
            etherscan_metadata,
            &config,
        );

        if self.json {
            println!("{}", serde_json::to_string(&json_results)?);
        }
        Ok(())
    }

    fn build_project(&self, config: &Config) -> Result<Bytes> {
        let project = config.project()?;
        let compiler = ProjectCompiler::new();

        let output = compiler.compile(&project)?;

        let artifact = output
            .find_contract(&self.contract)
            .ok_or_eyre("Build Error: Contract artifact not found locally")?;

        let local_bytecode = artifact
            .get_bytecode_object()
            .ok_or_eyre("Contract artifact does not have bytecode")?;

        let local_bytecode = match local_bytecode.as_ref() {
            BytecodeObject::Bytecode(bytes) => bytes,
            BytecodeObject::Unlinked(_) => {
                eyre::bail!("Unlinked bytecode is not supported for verification")
            }
        };

        Ok(local_bytecode.to_owned())
    }

    fn build_using_cache(&self, etherscan_settings: &Metadata, config: &Config) -> Option<Bytes> {
        let project = config.project().ok()?;
        let cache = project.read_cache_file().ok()?;
        let cached_artifacts = cache.read_artifacts::<CompactContractBytecode>().ok()?;

        for (key, value) in cached_artifacts {
            let name = self.contract.name.to_owned() + ".sol";
            let version = etherscan_settings.compiler_version.to_owned();
            // Ignores vyper
            if version.starts_with("vyper:") {
                return None;
            }
            // Parse etherscan version string
            let version =
                version.split('+').next().unwrap_or("").trim_start_matches('v').to_string();

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

    fn print_result(
        &self,
        res: (bool, Option<VerificationType>),
        bytecode_type: BytecodeType,
        json_results: &mut Vec<JsonResult>,
        etherscan_config: &Metadata,
        config: &Config,
    ) {
        if res.0 {
            if !self.json {
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
        } else if !res.0 && !self.json {
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
        } else if !res.0 && self.json {
            let json_res = JsonResult {
                bytecode_type,
                matched: false,
                verification_type: self.verification_type,
                message: Some(format!(
                    "{bytecode_type:?} code did not match - this may be due to varying compiler settings"
                )),
            };
            json_results.push(json_res);
        }
    }
}

/// Enum to represent the type of verification: `full` or `partial`.
/// Ref: <https://docs.sourcify.dev/docs/full-vs-partial-match/>
#[derive(Debug, Clone, clap::ValueEnum, Default, PartialEq, Eq, Serialize, Deserialize, Copy)]
pub enum VerificationType {
    #[default]
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "partial")]
    Partial,
}

impl FromStr for VerificationType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "full" => Ok(Self::Full),
            "partial" => Ok(Self::Partial),
            _ => eyre::bail!("Invalid verification type"),
        }
    }
}

impl From<VerificationType> for String {
    fn from(v: VerificationType) -> Self {
        match v {
            VerificationType::Full => "full".to_string(),
            VerificationType::Partial => "partial".to_string(),
        }
    }
}

impl fmt::Display for VerificationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "full"),
            Self::Partial => write!(f, "partial"),
        }
    }
}

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

fn try_match(
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
            .map(|matched| (matched, matched.then_some(VerificationType::Partial)))
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
