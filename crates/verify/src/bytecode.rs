use alloy_primitives::{Address, Uint, U256};
use alloy_providers::provider::TempProvider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use clap::{Parser, ValueHint};
use ethers_providers::Middleware;
use eyre::{OptionExt, Result};
use foundry_block_explorers::{contract::Metadata, Client};
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, read_constructor_args_file, LoadConfig},
};
use foundry_common::{
    compile::{ProjectCompiler, SkipBuildFilter, SkipBuildFilters},
    provider::alloy::ProviderBuilder,
    types::ToEthers,
};
use foundry_compilers::{
    artifacts::BytecodeObject, info::ContractInfo, Artifact, ProjectCompileOutput,
};
use foundry_config::{figment, impl_figment_convert, Chain, Config};
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER, executors::TracingExecutor, utils::configure_tx_env,
};
use revm_primitives::{db::Database, EnvWithHandlerCfg, HandlerCfg, SpecId};
use std::{ops::Range, path::PathBuf};
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

    /// Verfication Type: `full` or `partial`. Ref: https://docs.sourcify.dev/docs/full-vs-partial-match/
    #[clap(long, default_value = "full", value_name = "TYPE")]
    pub verification_type: String,

    #[clap(flatten)]
    pub etherscan_opts: EtherscanOpts,

    /// Skip building files whose names contain the given filter.
    ///
    /// `test` and `script` are aliases for `.t.sol` and `.s.sol`.
    #[arg(long, num_args(1..))]
    pub skip: Option<Vec<SkipBuildFilter>>,

    /// The path to the project's root directory.
    pub root: Option<PathBuf>,
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

        // if let Some(root) = self.root.as_ref() {
        //     dict.insert("root".to_string(), figment::value::Value::serialize(root)?);
        // }
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
        println!(
            "Verifying bytecode for contract {} at address {}",
            Paint::green(self.contract.name.clone()),
            Paint::green(self.address.to_string())
        );
        // If chain is not set, we try to get it from the RPC
        // If RPC is not set, the default chain is used
        let chain = match config.get_rpc_url() {
            Some(_) => {
                let chain_id = provider.get_chain_id().await?;
                // Convert to u64
                Chain::from(chain_id.to::<u64>())
            }
            None => config.chain.unwrap_or_default(),
        };

        // Set Etherscan options
        self.etherscan_opts.chain = Some(chain);
        self.etherscan_opts.key =
            config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);
        // Create etherscan client
        let etherscan = Client::new(chain, self.etherscan_opts.key.clone().unwrap())?;

        // Get the constructor args using `source_code` endpoint
        let source_code = etherscan.contract_source_code(self.address).await?;

        let constructor_args = match source_code.items.first() {
            Some(item) => item.constructor_arguments.clone(),
            None => {
                eyre::bail!("No source code found for contract at address {}", self.address);
            }
        };
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
        if provided_constructor_args != constructor_args.to_string() {
            eyre::bail!("Constructor args do not match, verification will fail.");
        }
        // Get creation tx hash
        let creation_data = etherscan.contract_creation_data(self.address).await?;

        let mut transaction = provider
            .get_transaction_by_hash(creation_data.transaction_hash)
            .await
            .or_else(|_| eyre::bail!("Couldn't fetch transaction from RPC"))?;
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash)
            .await
            .or_else(|_| eyre::bail!("Couldn't fetch transacrion receipt from RPC"))?;

        let receipt = match receipt {
            Some(receipt) => receipt,
            None => {
                eyre::bail!(
                    "Receipt not found for transaction hash {}",
                    creation_data.transaction_hash
                );
            }
        };
        // Extract creation code
        let maybe_creation_code = if receipt.contract_address == Some(self.address) {
            &transaction.input
        } else if transaction.to == Some(DEFAULT_CREATE2_DEPLOYER) {
            &transaction.input[32..]
        } else {
            eyre::bail!(
                "Could not extract the creation code for contract at address {}",
                self.address
            );
        };

        // Compile the project
        let output = self.build_project(&config)?;
        // let output = self.build_opts.run()?;
        let artifact = output
            .find_contract(&self.contract)
            .ok_or_eyre("Contract artifact not found locally")?;

        let local_bytecode = artifact
            .get_bytecode_object()
            .ok_or_eyre("Contract artifact does not have bytecode")?;

        let local_bytecode = match local_bytecode.as_ref() {
            BytecodeObject::Bytecode(bytes) => bytes,
            BytecodeObject::Unlinked(_) => {
                eyre::bail!("Unlinked bytecode is not supported for verification")
            }
        };

        // Etherscan compilation metadata
        let etherscan_metadata = source_code.items.first().unwrap();

        find_mismatch_in_settings(etherscan_metadata, &config)?;
        // Append constructor args to the local_bytecode
        let mut local_bytecode_vec = local_bytecode.to_vec();
        local_bytecode_vec.extend_from_slice(&constructor_args);

        // Cmp creation code with locally built bytecode and maybe_creation_code
        let res = try_match(
            local_bytecode_vec.as_slice(),
            maybe_creation_code,
            &constructor_args,
            &self.verification_type,
            false,
        )?;

        match res.0 {
            true => {
                println!(
                    "{} with status {}",
                    Paint::green("Creation code matched").bold(),
                    Paint::green(res.1.unwrap()).bold()
                );
            }
            false => {
                println!(
                    "{}",
                    Paint::red("Creation code did not match - This may be due to varying compiler settings").bold()
                );
            }
        }
        // Get contract creation block
        let simulation_block = match self.block {
            Some(block) => match block {
                BlockId::Number(BlockNumberOrTag::Number(block)) => block,
                _ => {
                    eyre::bail!("Invalid block number");
                }
            },
            None => {
                let provider = utils::get_provider(&config)?;
                let creation_block =
                    provider.get_transaction(creation_data.transaction_hash.to_ethers()).await?;
                match creation_block {
                    Some(tx) => tx.block_number.unwrap().as_u64(),
                    None => {
                        eyre::bail!(
                            "Failed to get block number of the contract creation tx, specify using
        the --block flag"
                        );
                    }
                }
            }
        };

        // Fork the chain at `simulation_block`

        let (mut fork_config, evm_opts) = config.clone().load_config_and_evm_opts()?;
        fork_config.fork_block_number = Some(simulation_block - 1);
        let (mut env, fork, _chain) =
            TracingExecutor::get_fork_material(&fork_config, evm_opts).await?;

        let mut executor = TracingExecutor::new(env.clone(), fork, Some(config.evm_version), false);
        env.block.number = U256::from(simulation_block);
        let block = provider.get_block(simulation_block.into(), true).await?;

        // Workaround for the NonceTooHigh issue as we're not simulating prior txs of the same
        // block.
        let prev_block_id = BlockId::Number(BlockNumberOrTag::Number(simulation_block - 1));
        let prev_block_nonce = provider
            .get_transaction_count(creation_data.contract_creator, Some(prev_block_id))
            .await?;
        transaction.nonce = Uint::<64, 1>::from(prev_block_nonce);

        if let Some(ref block) = block {
            env.block.timestamp = block.header.timestamp;
            env.block.coinbase = block.header.miner;
            env.block.difficulty = block.header.difficulty;
            env.block.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
            env.block.basefee = block.header.base_fee_per_gas.unwrap_or_default();
            env.block.gas_limit = block.header.gas_limit;
        }

        configure_tx_env(&mut env, &transaction);
        let env_with_handler =
            EnvWithHandlerCfg::new(Box::new(env.clone()), HandlerCfg::new(SpecId::LATEST));
        // Get the nonce of the contract creator at the block where the contract was created
        let deploy_result = match executor.deploy_with_env(env_with_handler, None) {
            Ok(result) => result,
            Err(error) => {
                eyre::bail!(
                    "Failed contract deploy transaction in block {} | Err: {:?}",
                    env.block.number,
                    error
                );
            }
        };

        // State commited using deploy_with_env, now get the runtime bytecode from the db.
        let fork_runtime_code = match executor.backend.basic(deploy_result.address)? {
            Some(account) => {
                if let Some(code) = account.code {
                    code
                } else {
                    eyre::bail!(
                        "Bytecode does not exist for contract deployed on fork at address {}",
                        deploy_result.address
                    );
                }
            }
            None => {
                eyre::bail!(
                    "Failed to get runtime code for contract deployed on fork at address {}",
                    deploy_result.address
                );
            }
        };

        let onchain_runtime_code = provider
            .get_code_at(
                self.address,
                Some(BlockId::Number(BlockNumberOrTag::Number(simulation_block))),
            )
            .await?;

        // Compare the runtime bytecode with the locally built bytecode
        let res = try_match(
            &fork_runtime_code.bytecode,
            &onchain_runtime_code,
            &constructor_args,
            &self.verification_type,
            true,
        )?;
        match res.0 {
            true => {
                println!(
                    "{} with status {}",
                    Paint::green("Runtime code matched").bold(),
                    Paint::green(res.1.unwrap()).bold()
                );
            }
            false => {
                println!(
                    "{}",
                    Paint::red(
                        "Runtime code did not match - This may be due to varying compiler settings"
                    )
                    .bold()
                );
            }
        }

        Ok(())
    }

    fn build_project(&self, config: &Config) -> Result<ProjectCompileOutput> {
        let project = config.project()?;
        let mut compiler = ProjectCompiler::new();

        if let Some(skip) = &self.skip {
            if !skip.is_empty() {
                compiler = compiler.filter(Box::new(SkipBuildFilters::new(skip.to_owned())?));
            }
        }
        let output = compiler.compile(&project)?;

        Ok(output)
    }
}

fn try_match(
    local_bytecode: &[u8],
    bytecode: &[u8],
    constructor_args: &[u8],
    match_type: &String,
    is_runtime: bool,
) -> Result<(bool, Option<String>)> {
    // 1. Try full match
    if match_type == "full" {
        if local_bytecode.starts_with(bytecode) {
            // Success => Full match
            Ok((true, Some("full".to_string())))
        } else {
            // Failure => Try partial match
            match try_partial_match(local_bytecode, bytecode, constructor_args, is_runtime) {
                Ok(true) => Ok((true, Some("partial".to_string()))),
                Ok(false) => Ok((false, None)),
                Err(e) => Err(e),
            }
        }
    } else {
        match try_partial_match(local_bytecode, bytecode, constructor_args, is_runtime) {
            Ok(true) => Ok((true, Some("partial".to_string()))),
            Ok(false) => Ok((false, None)),
            Err(e) => Err(e),
        }
    }
}

fn try_partial_match(
    mut local_bytecode: &[u8],
    mut bytecode: &[u8],
    constructor_args: &[u8],
    is_runtime: bool,
) -> Result<bool> {
    // 1. Check length of constructor args
    if constructor_args.is_empty() {
        // Assume metadata is at the end of the bytecode
        local_bytecode = extract_metadata_hash(local_bytecode)?;
        bytecode = extract_metadata_hash(bytecode)?;

        // Now compare the creation code and bytecode
        return Ok(local_bytecode.starts_with(bytecode));
    }

    if is_runtime {
        local_bytecode = extract_metadata_hash(local_bytecode)?;
        bytecode = extract_metadata_hash(bytecode)?;

        // Now compare the local code and bytecode
        return Ok(local_bytecode.starts_with(bytecode));
    }

    // If not runtime, extract constructor args from the end of the bytecode
    bytecode = &bytecode[..bytecode.len() - constructor_args.len()];
    local_bytecode = &local_bytecode[..local_bytecode.len() - constructor_args.len()];

    local_bytecode = extract_metadata_hash(local_bytecode)?;
    bytecode = extract_metadata_hash(bytecode)?;

    Ok(local_bytecode.starts_with(bytecode))
}

/// @dev This assumes that the metadata is at the end of the bytecode
fn extract_metadata_hash(bytecode: &[u8]) -> Result<&[u8]> {
    // Get the last two bytes of the bytecode to find the length of CBOR metadata
    let metadata_len = &bytecode[bytecode.len() - 2..];
    let metadata_len = u16::from_be_bytes([metadata_len[0], metadata_len[1]]);

    // Now discard the metadata from the bytecode
    Ok(&bytecode[..bytecode.len() - 2 - metadata_len as usize])
}

fn find_mismatch_in_settings(etherscan_settings: &Metadata, local_settings: &Config) -> Result<()> {
    println!("Scanning for mismatch in compiler settings...");
    if etherscan_settings.evm_version != local_settings.evm_version.to_string().to_lowercase() {
        println!(
            "{} - local: {} VS onchain: {}",
            Paint::red("EVM version mismatch").bold(),
            local_settings.evm_version,
            etherscan_settings.evm_version
        );
    }
    let local_optimizer: u64 = if local_settings.optimizer { 1 } else { 0 };
    if etherscan_settings.optimization_used != local_optimizer {
        println!(
            "{} - local: {} VS onchain: {}",
            Paint::red("Optimizer mismatch").bold(),
            local_settings.optimizer,
            etherscan_settings.optimization_used
        );
    }
    if etherscan_settings.runs != local_settings.optimizer_runs as u64 {
        println!(
            "{} - local: {} VS onchain: {}",
            Paint::red("Optimizer runs mismatch").bold(),
            local_settings.optimizer_runs,
            etherscan_settings.runs
        );
    }

    // TODO: Compiler Version Check

    Ok(())
}
