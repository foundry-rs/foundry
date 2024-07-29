use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{hex, Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use clap::{Parser, ValueHint};
use eyre::{OptionExt, Result};
use foundry_block_explorers::{contract::Metadata, Client};
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, read_constructor_args_file, LoadConfig},
};
use foundry_common::{abi::encode_args, compile::ProjectCompiler, provider::ProviderBuilder};
use foundry_compilers::{
    artifacts::{BytecodeHash, CompactContractBytecode, EvmVersion},
    info::ContractInfo,
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
        num_args(1..),
        conflicts_with_all = &["constructor_args_path", "encoded_constructor_args"],
        value_name = "ARGS",
    )]
    pub constructor_args: Option<Vec<String>>,

    /// The ABI-encoded constructor arguments.
    #[arg(
        long,
        conflicts_with_all = &["constructor_args_path", "constructor_args"],
        value_name = "HEX",
    )]
    pub encoded_constructor_args: Option<String>,

    /// The path to a file containing the constructor arguments.
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        conflicts_with_all = &["constructor_args", "encoded_constructor_args"]
    )]
    pub constructor_args_path: Option<PathBuf>,

    /// The rpc url to use for verification.
    #[clap(short = 'r', long, value_name = "RPC_URL", env = "ETH_RPC_URL")]
    pub rpc_url: Option<String>,

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
        let mut dict = self.etherscan_opts.dict();
        if let Some(block) = &self.block {
            dict.insert("block".into(), figment::value::Value::serialize(block)?);
        }
        if let Some(rpc_url) = &self.rpc_url {
            dict.insert("eth_rpc_url".into(), rpc_url.to_string().into());
        }

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

        // Get creation tx hash
        let creation_data = etherscan.contract_creation_data(self.address).await?;

        trace!(creation_tx_hash = ?creation_data.transaction_hash);
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

        // Get the constructor args using `source_code` endpoint
        let source_code = etherscan.contract_source_code(self.address).await?;

        // Check if the contract name matches
        let name = source_code.items.first().map(|item| item.contract_name.to_owned());
        if name.as_ref() != Some(&self.contract.name) {
            eyre::bail!("Contract name mismatch");
        }

        // Obtain Etherscan compilation metadata
        let etherscan_metadata = source_code.items.first().unwrap();

        // Obtain local artifact
        let artifact =
            if let Ok(local_bytecode) = self.build_using_cache(etherscan_metadata, &config) {
                trace!("using cache");
                local_bytecode
            } else {
                self.build_project(&config)?
            };

        let local_bytecode = artifact
            .bytecode
            .and_then(|b| b.into_bytes())
            .ok_or_eyre("Unlinked bytecode is not supported for verification")?;

        // Get the constructor args from etherscan
        let mut constructor_args = if let Some(args) = source_code.items.first() {
            args.constructor_arguments.clone()
        } else {
            eyre::bail!("No constructor arguments found for contract at address {}", self.address);
        };

        // Get and encode user provided constructor args
        let provided_constructor_args = if let Some(path) = self.constructor_args_path.to_owned() {
            // Read from file
            Some(read_constructor_args_file(path)?)
        } else {
            self.constructor_args.to_owned()
        }
        .map(|args| {
            if let Some(constructor) = artifact.abi.as_ref().and_then(|abi| abi.constructor()) {
                if constructor.inputs.len() != args.len() {
                    eyre::bail!(
                        "Mismatch of constructor arguments length. Expected {}, got {}",
                        constructor.inputs.len(),
                        args.len()
                    );
                }
                encode_args(&constructor.inputs, &args)
                    .map(|args| DynSolValue::Tuple(args).abi_encode())
            } else {
                Ok(Vec::new())
            }
        })
        .transpose()?
        .or(self.encoded_constructor_args.to_owned().map(hex::decode).transpose()?);

        if let Some(provided) = provided_constructor_args {
            constructor_args = provided.into();
        } else {
            // In some cases, Etherscan will return incorrect constructor arguments. If this
            // happens, try extracting arguments ourselves.
            if !maybe_creation_code.ends_with(&constructor_args) {
                trace!("mismatch of constructor args with etherscan");
                // If local bytecode is longer than on-chain one, this is probably not a match.
                if maybe_creation_code.len() >= local_bytecode.len() {
                    constructor_args =
                        Bytes::copy_from_slice(&maybe_creation_code[local_bytecode.len()..]);
                    trace!(
                        "setting constructor args to latest {} bytes of bytecode",
                        constructor_args.len()
                    );
                }
            }
        }

        // If bytecode_hash is disabled then its always partial verification
        let has_metadata = config.bytecode_hash == BytecodeHash::None;

        // Append constructor args to the local_bytecode
        trace!(%constructor_args);
        let mut local_bytecode_vec = local_bytecode.to_vec();
        local_bytecode_vec.extend_from_slice(&constructor_args);

        // Cmp creation code with locally built bytecode and maybe_creation_code
        let match_type = match_bytecodes(
            local_bytecode_vec.as_slice(),
            maybe_creation_code,
            &constructor_args,
            false,
            has_metadata,
        );

        let mut json_results: Vec<JsonResult> = vec![];
        self.print_result(
            match_type,
            BytecodeType::Creation,
            &mut json_results,
            etherscan_metadata,
            &config,
        );

        // If the creation code does not match, the runtime also won't match. Hence return.
        if match_type.is_none() {
            self.print_result(
                None,
                BytecodeType::Runtime,
                &mut json_results,
                etherscan_metadata,
                &config,
            );
            if self.json {
                println!("{}", serde_json::to_string(&json_results)?);
            }
            return Ok(());
        }

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
            TracingExecutor::new(env.clone(), fork, Some(fork_config.evm_version), false, false);
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

        // Replace the `input` with local creation code in the creation tx.
        if let Some(to) = transaction.to {
            if to == DEFAULT_CREATE2_DEPLOYER {
                let mut input = transaction.input[..32].to_vec(); // Salt
                input.extend_from_slice(&local_bytecode_vec);
                transaction.input = Bytes::from(input);

                // Deploy default CREATE2 deployer
                executor.deploy_create2_deployer()?;
            }
        } else {
            transaction.input = Bytes::from(local_bytecode_vec);
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

        // Compare the onchain runtime bytecode with the runtime code from the fork.
        let match_type = match_bytecodes(
            &fork_runtime_code.original_bytes(),
            &onchain_runtime_code,
            &constructor_args,
            true,
            has_metadata,
        );

        self.print_result(
            match_type,
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

    fn build_project(&self, config: &Config) -> Result<CompactContractBytecode> {
        let project = config.project()?;
        let compiler = ProjectCompiler::new();

        let mut output = compiler.compile(&project)?;

        let artifact = output
            .remove_contract(&self.contract)
            .ok_or_eyre("Build Error: Contract artifact not found locally")?;

        Ok(artifact.into_contract_bytecode())
    }

    fn build_using_cache(
        &self,
        etherscan_settings: &Metadata,
        config: &Config,
    ) -> Result<CompactContractBytecode> {
        let project = config.project()?;
        let cache = project.read_cache_file()?;
        let cached_artifacts = cache.read_artifacts::<CompactContractBytecode>()?;

        for (key, value) in cached_artifacts {
            let name = self.contract.name.to_owned() + ".sol";
            let version = etherscan_settings.compiler_version.to_owned();
            // Ignores vyper
            if version.starts_with("vyper:") {
                eyre::bail!("Vyper contracts are not supported")
            }
            // Parse etherscan version string
            let version =
                version.split('+').next().unwrap_or("").trim_start_matches('v').to_string();

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

        eyre::bail!("couldn't find cached artifact for contract {}", self.contract.name)
    }

    fn print_result(
        &self,
        res: Option<VerificationType>,
        bytecode_type: BytecodeType,
        json_results: &mut Vec<JsonResult>,
        etherscan_config: &Metadata,
        config: &Config,
    ) {
        if let Some(res) = res {
            if !self.json {
                println!(
                    "{} with status {}",
                    format!("{bytecode_type:?} code matched").green().bold(),
                    res.green().bold()
                );
            } else {
                let json_res = JsonResult { bytecode_type, match_type: Some(res), message: None };
                json_results.push(json_res);
            }
        } else if !self.json {
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
    pub match_type: Option<VerificationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn match_bytecodes(
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
