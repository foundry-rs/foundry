use alloy_primitives::Address;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use clap::{Parser, ValueHint};
use ethers_core::types::Eip1559TransactionRequest;
use ethers_providers::Middleware;
use eyre::{OptionExt, Result};
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::{CoreBuildArgs, EtherscanOpts},
    utils::{self, LoadConfig},
};
use foundry_common::{
    compile::{ProjectCompiler, SkipBuildFilter, SkipBuildFilters},
    types::ToEthers,
};
use foundry_compilers::{
    artifacts::BytecodeObject, info::ContractInfo, Artifact, ProjectCompileOutput,
};
use foundry_config::{figment, merge_impl_figment_convert, Config};
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER,
    fork::{CreateFork, MultiFork},
    opts::EvmOpts,
};
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
        let provider = utils::get_provider(&config)?;

        tracing::info!("Verifying contract at address {}", self.address);
        // If chain is not set, we try to get it from the RPC
        // If RPC is not set, the default chain is used

        let chain = match config.get_rpc_url() {
            Some(_) => utils::get_chain(config.chain, provider.clone()).await?,
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
            Some(item) => {
                tracing::info!("Contract Name: {:?}", item.contract_name);
                tracing::info!("Compiler Version {:?}", item.compiler_version);
                tracing::info!("EVM Version {:?}", item.evm_version);
                tracing::info!("Optimization {:?}", item.optimization_used);
                tracing::info!("Runs {:?}", item.runs);
                item.constructor_arguments.clone()
            }
            None => {
                eyre::bail!("No source code found for contract at address {}", self.address);
            }
        };

        tracing::info!("Constructor args: {:?}", constructor_args);

        // Get creation tx hash
        let creation_data = etherscan.contract_creation_data(self.address).await?;

        tracing::info!("Creation data: {:?}", creation_data);
        let transaction = provider
            .get_transaction(creation_data.transaction_hash.to_ethers())
            .await?
            .ok_or_eyre("Couldn't fetch transaction data from RPC")?;
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash.to_ethers())
            .await?
            .ok_or_eyre("Couldn't fetch transaction receipt from RPC")?;

        // Extract creation code
        let maybe_creation_code = if receipt.contract_address == Some(self.address.to_ethers()) {
            &transaction.input
        } else if transaction.to == Some(DEFAULT_CREATE2_DEPLOYER.to_ethers()) {
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
        let res = try_match(maybe_creation_code, bytecode, self.verification_type)?;
        tracing::info!("Match: {} | Type: {:?}", res.0, res.1);

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
        let fork = CreateFork {
            enable_caching: false,
            url: self.rpc_url.unwrap_or_default(),
            env: Default::default(),
            evm_opts: EvmOpts { fork_block_number: Some(simulation_block), ..Default::default() },
        };
        tracing::info!("Forking the chain at block {}", simulation_block);
        let multi_fork = MultiFork::spawn();
        let (fork_id, _shared_backend, _env) = multi_fork.create_fork(fork)?;

        tracing::info!("Created fork with id {}", fork_id);
        // TODO: @Yash
        // Deploy the contract on the forked chain
        let fork_rpc_url = multi_fork.get_fork_url(fork_id)?;
        let mut fork_config = config.clone();
        fork_config.eth_rpc_url = fork_rpc_url;
        let fork_provider = utils::get_provider(&fork_config)?;

        let tx = Eip1559TransactionRequest {
            from: Some(creation_data.contract_creator.to_ethers()),
            data: Some(bytecode.to_owned().to_ethers()),
            ..Default::default()
        };
        // @mattsse - Need some help here.
        let pending_tx = fork_provider.send_transaction(tx, None).await?;

        let tx_receipt = pending_tx.confirmations(1).await.ok().flatten().ok_or_eyre(
            "Failed to deploy locally built bytecode on the forked chain to verify runtime bytecode",
        )?;

        tracing::info!("Deployed contract at address {}", tx_receipt.contract_address.unwrap());
        // Get onchain runtime bytecode
        let _runtime_code = provider.get_code(self.address.to_ethers(), None).await?;

        // Get fork runtime bytecode
        let _fork_runtime_code = fork_provider.get_code(self.address.to_ethers(), None).await?;
        // Cmp runtime bytecode with onchain deployed bytecode
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
    creation_code: &[u8],
    bytecode: &[u8],
    match_type: String,
) -> Result<(bool, Option<String>)> {
    if match_type == "full" {
        if creation_code.starts_with(bytecode) {
            Ok((true, Some("full".to_string())))
        } else {
            match try_partial_match(creation_code, bytecode) {
                Ok(true) => Ok((true, Some("partial".to_string()))),
                Ok(false) => Ok((false, None)),
                Err(e) => Err(e),
            }
        }
    } else {
        match try_partial_match(creation_code, bytecode) {
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
