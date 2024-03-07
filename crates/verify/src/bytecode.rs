use alloy_primitives::{Address, U256};
use alloy_providers::provider::TempProvider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use clap::{Parser, ValueHint};
use ethers_providers::Middleware;
use eyre::{OptionExt, Result};
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::{CoreBuildArgs, EtherscanOpts},
    utils::{self, LoadConfig},
};
use foundry_common::{
    compile::{ProjectCompiler, SkipBuildFilter, SkipBuildFilters},
    provider::alloy::ProviderBuilder,
    types::ToEthers,
};
use foundry_compilers::{
    artifacts::BytecodeObject, info::ContractInfo, Artifact, ProjectCompileOutput,
};
use foundry_config::{figment, merge_impl_figment_convert, Chain, Config};
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER, executors::TracingExecutor, utils::configure_tx_env,
};
use revm_primitives::db::Database;
use std::path::PathBuf;

merge_impl_figment_convert!(VerifyBytecodeArgs, build_opts);
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

    /// The build options to use for verification.
    #[clap(flatten)]
    pub build_opts: CoreBuildArgs,

    /// Print compiled contract names.
    #[arg(long)]
    pub names: bool,

    /// Print compiled contract sizes.
    #[arg(long)]
    pub sizes: bool,

    /// Skip building files whose names contain the given filter.
    ///
    /// `test` and `script` are aliases for `.t.sol` and `.s.sol`.
    #[arg(long, num_args(1..))]
    pub skip: Option<Vec<SkipBuildFilter>>,

    /// Output the compilation errors in the json format.
    /// This is useful when you want to use the output in other tools.
    #[arg(long, conflicts_with = "silent")]
    pub format_json: bool,
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
        let config = self.load_config_emit_warnings();
        // let provider = utils::get_provider(&config)?;
        let provider = ProviderBuilder::new(&config.get_rpc_url_or_localhost_http()?).build()?;
        tracing::info!("Checking bytecode contract at address {}", self.address);
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

        tracing::info!("Constructor args: {:?}", constructor_args);

        // Get creation tx hash
        let creation_data = etherscan.contract_creation_data(self.address).await?;

        let transaction = provider
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

        let bytecode = artifact
            .get_bytecode_object()
            .ok_or_eyre("Contract artifact does not have bytecode")?;

        let bytecode = match bytecode.as_ref() {
            BytecodeObject::Bytecode(bytes) => bytes,
            BytecodeObject::Unlinked(_) => {
                eyre::bail!("Unlinked bytecode is not supported for verification")
            }
        };

        // Cmp creation code with locally built bytecode and maybe_creation_code
        let res = try_match(maybe_creation_code, bytecode, self.verification_type.clone())?;
        tracing::info!("Creation code match: {} | Type: {:?}", res.0, res.1);

        // TODO: @Yash
        // Fork the chain at `simulation_block`, deploy the contract and compare the runtime
        // bytecode.
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
        if let Some(ref block) = block {
            env.block.timestamp = block.header.timestamp;
            env.block.coinbase = block.header.miner;
            env.block.difficulty = block.header.difficulty;
            env.block.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
            env.block.basefee = block.header.base_fee_per_gas.unwrap_or_default();
            env.block.gas_limit = block.header.gas_limit;
        }

        configure_tx_env(&mut env, &transaction);
        let deploy_result = match executor.deploy_with_env(env.clone(), None) {
            Ok(result) => result,
            Err(_error) => {
                eyre::bail!("Failed contract deploy transaction in block {}", env.block.number);
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
        let res =
            try_match(&fork_runtime_code.bytecode, &onchain_runtime_code, self.verification_type)?;
        tracing::info!("Runtime code match: {} | Type: {:?}", res.0, res.1);
        Ok(())
    }

    fn build_project(&self, config: &Config) -> Result<ProjectCompileOutput> {
        let project = config.project()?;
        let mut compiler = ProjectCompiler::new()
            .print_names(self.names)
            .print_sizes(self.sizes)
            .quiet(self.format_json)
            .bail(!self.format_json);

        if let Some(skip) = &self.skip {
            if !skip.is_empty() {
                compiler = compiler.filter(Box::new(SkipBuildFilters::new(skip.to_owned())?));
            }
        }
        let output = compiler.compile(&project)?;

        if self.format_json {
            println!("{}", serde_json::to_string_pretty(&output.clone().output())?);
        }
        Ok(output)
    }
}

fn try_match(
    local_bytecode: &[u8],
    bytecode: &[u8],
    match_type: String,
) -> Result<(bool, Option<String>)> {
    if match_type == "full" {
        if local_bytecode.starts_with(bytecode) {
            Ok((true, Some("full".to_string())))
        } else {
            match try_partial_match(local_bytecode, bytecode) {
                Ok(true) => Ok((true, Some("partial".to_string()))),
                Ok(false) => Ok((false, None)),
                Err(e) => Err(e),
            }
        }
    } else {
        match try_partial_match(local_bytecode, bytecode) {
            Ok(true) => Ok((true, Some("partial".to_string()))),
            Ok(false) => Ok((false, None)),
            Err(e) => Err(e),
        }
    }
}

fn try_partial_match(creation_code: &[u8], bytecode: &[u8]) -> Result<bool> {
    // Get the last two bytes of the creation code to find the length of CBOR metadata
    let creation_code_metadata_len = &creation_code[creation_code.len() - 2..];
    let metadata_len =
        u16::from_be_bytes([creation_code_metadata_len[0], creation_code_metadata_len[1]]);

    // Now discard the metadata from the creation code
    let creation_code = &creation_code[..creation_code.len() - 2 - metadata_len as usize];

    // Do the same for the bytecode
    let bytecode_metadata_len = &bytecode[bytecode.len() - 2..];
    let metadata_len = u16::from_be_bytes([bytecode_metadata_len[0], bytecode_metadata_len[1]]);

    let bytecode = &bytecode[..bytecode.len() - 2 - metadata_len as usize];

    // Now compare the creation code and bytecode
    if creation_code.starts_with(bytecode) {
        Ok(true)
    } else {
        Ok(false)
    }
}
