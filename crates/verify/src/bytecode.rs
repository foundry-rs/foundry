//! The `forge verify-bytecode` command.
use crate::{
    etherscan::EtherscanVerificationProvider,
    utils::{
        BytecodeType, JsonResult, check_and_encode_args, check_explorer_args,
        load_fork_config_and_evm_opts, maybe_predeploy_contract, synthetic_deployment_context,
        validate_encoded_constructor_args,
    },
    verify::VerifierArgs,
};
use alloy_consensus::Transaction as ConsensusTransaction;
use alloy_network::{AnyNetwork, AnyRpcBlock};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, hex};
use alloy_provider::{
    Provider,
    ext::TraceApi,
    network::{BlockResponse, ReceiptResponse, TransactionResponse, primitives::BlockTransactions},
};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    trace::parity::{Action, CreateAction, CreateOutput, TraceOutput},
};
use clap::{Parser, ValueHint};
use eyre::{Context, OptionExt, Result};
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, LoadConfig, read_constructor_args_file},
};
use foundry_common::{
    SYSTEM_TRANSACTION_TYPE, is_known_system_sender, provider::ProviderBuilder, shell,
};
use foundry_compilers::info::ContractInfo;
use foundry_config::{Chain, Config, figment, impl_figment_convert};
#[cfg(feature = "base")]
use foundry_evm::core::evm::BaseEvmNetwork;
#[cfg(feature = "monad")]
use foundry_evm::core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER,
    core::{
        FoundryChain, FoundryTransaction as _,
        env::FromAnyRpcTransaction as _,
        evm::{
            BlockContext, ChainFor, EthEvmNetwork, EvmEnvFor, FoundryEvmNetwork, TempoEvmNetwork,
            TxEnvFor,
        },
    },
    executors::{EvmError, ExecutorBuilder, TracingExecutor},
    opts::{EvmOpts, ForkEndpointIdentity},
    utils::apply_chain_specific_tx_replay_env_changes_for_chain,
};
use foundry_evm_networks::NetworkVariant;
use revm::{context::Block as _, state::AccountInfo};
use std::{path::PathBuf, pin::Pin};

impl_figment_convert!(VerifyBytecodeArgs);

/// CLI arguments for `forge verify-bytecode`.
#[derive(Clone, Debug, Parser)]
pub struct VerifyBytecodeArgs {
    /// The address of the contract to verify.
    pub address: Address,

    /// The contract identifier in the form `<path>:<contractname>`.
    pub contract: ContractInfo,

    /// The block at which the bytecode should be verified.
    #[arg(long, value_name = "BLOCK")]
    pub block: Option<BlockId>,

    /// The constructor args to generate the creation code.
    #[arg(
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
    #[arg(short = 'r', long, value_name = "RPC_URL", env = "ETH_RPC_URL")]
    pub rpc_url: Option<String>,

    /// Specify the network for correct encoding.
    #[arg(long, short, num_args = 1, value_name = "NETWORK")]
    pub network: Option<NetworkVariant>,

    /// Etherscan options.
    #[command(flatten)]
    pub etherscan: EtherscanOpts,

    /// Verifier options.
    #[command(flatten)]
    pub verifier: VerifierArgs,

    /// Set pre-linked libraries.
    #[arg(long, help_heading = "Linker options")]
    pub libraries: Vec<String>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Ignore verification for creation or runtime bytecode.
    #[arg(long, value_name = "BYTECODE_TYPE")]
    pub ignore: Option<BytecodeType>,
}

impl figment::Provider for VerifyBytecodeArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Bytecode Provider")
    }

    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        let mut dict = self.etherscan.dict();

        if let Some(api_key) = &self.verifier.verifier_api_key {
            dict.insert("etherscan_api_key".into(), api_key.as_str().into());
        }

        if let Some(block) = &self.block {
            dict.insert("block".into(), figment::value::Value::serialize(block)?);
        }
        if let Some(rpc_url) = &self.rpc_url {
            dict.insert("eth_rpc_url".into(), rpc_url.clone().into());
        }

        Ok(figment::value::Map::from([(Config::selected_profile(), dict)]))
    }
}

impl VerifyBytecodeArgs {
    fn configured_network(
        cli_network: Option<NetworkVariant>,
        config: &Config,
    ) -> Option<NetworkVariant> {
        cli_network.or_else(|| {
            config.networks.has_network_selection().then(|| config.networks.execution_network())
        })
    }

    async fn endpoint_identity(config: &Config) -> Result<Option<ForkEndpointIdentity>> {
        let (_, mut evm_opts) = load_fork_config_and_evm_opts(config)?;
        if evm_opts.fork_url.is_none() {
            evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        }
        evm_opts.infer_network_from_fork().await?;
        Ok(evm_opts.fork_endpoint)
    }

    async fn ensure_endpoint_identity_unchanged(
        config: &Config,
        expected: Option<&ForkEndpointIdentity>,
    ) -> Result<()> {
        let Some(expected) = expected else { return Ok(()) };
        let current = Self::endpoint_identity(config).await?.ok_or_else(|| {
            eyre::eyre!("RPC endpoint identity disappeared while verify-bytecode was running")
        })?;
        Self::validate_endpoint_identity(expected, &current)
    }

    fn validate_endpoint_identity(
        expected: &ForkEndpointIdentity,
        current: &ForkEndpointIdentity,
    ) -> Result<()> {
        if current != expected {
            eyre::bail!(
                "RPC endpoint identity changed while verify-bytecode was running; retry against \
                 a stable endpoint"
            );
        }
        Ok(())
    }

    fn apply_endpoint_expectation(
        evm_opts: &mut EvmOpts,
        endpoint_identity: Option<&ForkEndpointIdentity>,
        network_was_inferred: bool,
    ) {
        if let Some(identity) = endpoint_identity {
            evm_opts.expect_fork_endpoint(identity.clone(), network_was_inferred);
        }
    }

    fn effective_network(
        configured: Option<NetworkVariant>,
        endpoint_identity: Option<&ForkEndpointIdentity>,
    ) -> NetworkVariant {
        configured
            .or_else(|| endpoint_identity.map(|identity| identity.network))
            .unwrap_or(NetworkVariant::Ethereum)
    }

    fn materialize_execution_network(
        config: &mut Config,
        endpoint_identity: Option<&ForkEndpointIdentity>,
    ) -> NetworkVariant {
        let configured = Self::configured_network(None, config);
        let network = Self::effective_network(configured, endpoint_identity);
        if configured.is_none() {
            config.networks = if let Some(identity) = endpoint_identity {
                config.networks.with_rpc_profile(identity.network_profile)
            } else {
                network.into()
            };
        }
        network
    }

    fn explorer_chain(
        configured: Option<Chain>,
        endpoint_identity: Option<&ForkEndpointIdentity>,
    ) -> Option<Chain> {
        configured.or_else(|| endpoint_identity.map(|identity| identity.source_chain_id.into()))
    }

    /// Run the `verify-bytecode` command to verify the bytecode onchain against the locally built
    /// bytecode.
    pub async fn run(mut self) -> Result<()> {
        let mut config = self.load_config()?;
        config.libraries.append(&mut self.libraries);

        if let Some(network) = self.network {
            config.networks = network.into();
        }
        let network_was_inferred = Self::configured_network(None, &config).is_none();
        let endpoint_identity = Self::endpoint_identity(&config).await?;
        let network = Self::materialize_execution_network(&mut config, endpoint_identity.as_ref());

        match network {
            NetworkVariant::Ethereum => {
                Box::pin(self.run_with_network_and_config::<EthEvmNetwork>(
                    config,
                    endpoint_identity,
                    network_was_inferred,
                    replay_block_transactions::<EthEvmNetwork>,
                    ExecutorBuilder::<EthEvmNetwork>::new(),
                ))
                .await
            }
            #[cfg(feature = "base")]
            NetworkVariant::Base => {
                Box::pin(self.run_with_network_and_config::<BaseEvmNetwork>(
                    config,
                    endpoint_identity,
                    network_was_inferred,
                    replay_block_transactions::<BaseEvmNetwork>,
                    ExecutorBuilder::<BaseEvmNetwork>::new(),
                ))
                .await
            }
            #[cfg(feature = "optimism")]
            NetworkVariant::Optimism => {
                Box::pin(self.run_with_network_and_config::<OpEvmNetwork>(
                    config,
                    endpoint_identity,
                    network_was_inferred,
                    replay_block_transactions::<OpEvmNetwork>,
                    ExecutorBuilder::<OpEvmNetwork>::new(),
                ))
                .await
            }
            NetworkVariant::Tempo => {
                Box::pin(self.run_with_network_and_config::<TempoEvmNetwork>(
                    config,
                    endpoint_identity,
                    network_was_inferred,
                    replay_block_transactions::<TempoEvmNetwork>,
                    ExecutorBuilder::<TempoEvmNetwork>::new(),
                ))
                .await
            }
            #[cfg(feature = "monad")]
            NetworkVariant::Monad => {
                Box::pin(self.run_with_network_and_config::<MonadEvmNetwork>(
                    config,
                    endpoint_identity,
                    network_was_inferred,
                    replay_monad_block_transactions,
                    ExecutorBuilder::<MonadEvmNetwork>::new(),
                ))
                .await
            }
        }
    }

    async fn run_with_network_and_config<FEN>(
        mut self,
        config: Config,
        endpoint_identity: Option<ForkEndpointIdentity>,
        network_was_inferred: bool,
        replay_block: ReplayBlockFn<FEN>,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<()>
    where
        FEN: FoundryEvmNetwork,
    {
        // Setup
        // `AnyNetwork` rather than `FEN::Network`: chains such as Arbitrum and Celo put
        // transaction types the strict Ethereum envelope cannot decode into every block, which
        // would fail the full block fetches below for the whole chain. Execution still uses `FEN`.
        let provider = ProviderBuilder::<AnyNetwork>::from_config(&config)?.build()?;

        // If chain is not set, we try to get it from the RPC.
        // If RPC is not set, the default chain is used.
        let chain = match (
            Self::explorer_chain(config.chain, endpoint_identity.as_ref()),
            config.get_rpc_url(),
        ) {
            (Some(chain), _) => chain,
            (None, Some(_)) => utils::get_chain::<AnyNetwork, _>(None, &provider).await?,
            (None, None) => Default::default(),
        };

        // Set Etherscan options.
        self.etherscan.chain = Some(chain);
        self.etherscan.key = config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);

        // Whether a block explorer is configured for this chain. Client setup errors are only
        // treated as "no explorer available" when no usable verifier, verifier URL, or resolved
        // API key is configured.
        let has_explorer_config = self.verifier.verifier.is_some()
            || self.verifier.verifier_url.is_some()
            || self.verifier.verifier_api_key.is_some()
            || self.etherscan.key.is_some();

        // Etherscan client. May be unavailable (e.g. unknown chain, missing configuration), in
        // which case verification proceeds with local data only.
        let etherscan = match EtherscanVerificationProvider.client(
            &self.etherscan,
            &self.verifier,
            &config,
        ) {
            Ok(client) => Some(client),
            Err(err) => {
                if has_explorer_config {
                    return Err(err);
                }
                if !shell::is_json() {
                    sh_warn!(
                        "Failed to create a block explorer client: {err}. Continuing with the local project configuration."
                    )?;
                }
                None
            }
        };

        // Get the bytecode at the address, bailing if it doesn't exist.
        let code = provider.get_code_at(self.address).await?;
        Self::ensure_endpoint_identity_unchanged(&config, endpoint_identity.as_ref()).await?;
        if code.is_empty() {
            eyre::bail!("No bytecode found at address {}", self.address);
        }

        if !shell::is_json() {
            sh_status!(
                "Verifying bytecode for contract {} at address {}",
                self.contract.name,
                self.address
            )?;
        }

        let mut json_results: Vec<JsonResult> = vec![];

        // Get creation tx hash. An unavailable explorer (missing API key, unsupported chain,
        // unverified contract, etc.) must not prevent verification against a local build: fall
        // back to verifying the runtime bytecode only.
        // See <https://github.com/foundry-rs/foundry/issues/13479>.
        let (creation_data, maybe_predeploy) = match &etherscan {
            Some(etherscan) => {
                let creation_data = etherscan.contract_creation_data(self.address).await;

                // Check if contract is a predeploy
                match maybe_predeploy_contract(creation_data) {
                    Ok(res) => res,
                    Err(err) => {
                        if has_explorer_config {
                            return Err(err);
                        }
                        if !shell::is_json() {
                            sh_warn!(
                                "Failed to fetch creation data from the block explorer: {err}"
                            )?;
                        }
                        (None, false)
                    }
                }
            }
            None => (None, false),
        };

        trace!(maybe_predeploy = ?maybe_predeploy);

        // Get the constructor args using `source_code` endpoint.
        let source_code = match &etherscan {
            Some(etherscan) => match etherscan.contract_source_code(self.address).await {
                Ok(source_code) => {
                    if let Some(metadata) = source_code.items.first() {
                        // Check if the contract name matches.
                        if metadata.contract_name != self.contract.name {
                            eyre::bail!("Contract name mismatch");
                        }
                        Some(source_code)
                    } else {
                        if !shell::is_json() {
                            sh_warn!(
                                "Block explorer returned no source metadata. Continuing with the local project configuration; compiler settings mismatches will not be reported."
                            )?;
                        }
                        None
                    }
                }
                Err(err) => {
                    if has_explorer_config {
                        return Err(err.into());
                    }
                    if !shell::is_json() {
                        sh_warn!(
                            "Failed to fetch contract source code from the block explorer: {err}. Continuing with the local project configuration; compiler settings mismatches will not be reported."
                        )?;
                    }
                    None
                }
            },
            None => None,
        };

        // Obtain Etherscan compilation metadata.
        let etherscan_metadata = source_code.as_ref().and_then(|source| source.items.first());

        // Obtain local artifact
        let artifact = crate::utils::build_project(&self, &config)?;

        // Get local bytecode (creation code)
        let local_bytecode = artifact
            .bytecode
            .as_ref()
            .and_then(|b| b.to_owned().into_bytes())
            .ok_or_eyre("Unlinked bytecode is not supported for verification")?;

        // Get and encode user provided constructor args
        let provided_constructor_args = if let Some(encoded) = &self.encoded_constructor_args {
            Some(validate_encoded_constructor_args(&artifact, hex::decode(encoded)?)?)
        } else {
            if let Some(path) = self.constructor_args_path.clone() {
                // Read from file.
                Some(read_constructor_args_file(path)?)
            } else {
                self.constructor_args.clone()
            }
            .map(|args| check_and_encode_args(&artifact, args))
            .transpose()?
        };

        let args_from_user = provided_constructor_args.is_some();
        let mut constructor_args = if let Some(provided) = provided_constructor_args {
            provided.into()
        } else if let Some(source_code) = &source_code {
            // If no constructor args were provided, try to retrieve them from the explorer.
            check_explorer_args(source_code)?
        } else {
            Bytes::new()
        };

        // This fails only when the contract expects constructor args but NONE were provided OR
        // retrieved from explorer (in case of predeploys).
        crate::utils::check_args_len(&artifact, &constructor_args)?;

        // Without creation data (predeploys, or the explorer being unavailable), the creation
        // code cannot be verified. Verify the runtime bytecode instead by deploying the local
        // creation code and comparing the resulting runtime code with the onchain one.
        if creation_data.is_none() {
            if !shell::is_json() {
                if maybe_predeploy {
                    sh_warn!(
                        "Attempting to verify predeployed contract at {:?}. Ignoring creation code verification.",
                        self.address
                    )?;
                } else {
                    sh_warn!("Creation data is unavailable. Ignoring creation code verification.")?;
                }
            }

            // Without creation data there is nothing else to verify when the runtime bytecode is
            // ignored.
            if self.ignore.is_some_and(|b| b.is_runtime()) {
                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&json_results)?)?;
                }
                return Ok(());
            }

            let deploy_block = if maybe_predeploy {
                // Deploy at genesis
                0_u64
            } else {
                match self.block {
                    Some(BlockId::Number(BlockNumberOrTag::Number(block))) => block,
                    Some(_) => {
                        eyre::bail!("Invalid block number");
                    }
                    None => provider.get_block_number().await?,
                }
            };

            // Append constructor args to the local_bytecode.
            trace!(%constructor_args);
            let mut local_bytecode_vec = local_bytecode.to_vec();
            local_bytecode_vec.extend_from_slice(&constructor_args);

            let deploy_block_info = provider.get_block(deploy_block.into()).full().await?;
            let (mut fork_config, mut evm_opts) = load_fork_config_and_evm_opts(&config)?;
            Self::apply_endpoint_expectation(
                &mut evm_opts,
                endpoint_identity.as_ref(),
                network_was_inferred,
            );
            let (evm_env, _, mut executor) = crate::utils::get_tracing_executor::<FEN>(
                &mut fork_config,
                deploy_block,
                deploy_block,
                deploy_block_info.as_ref(),
                evm_opts,
                executor_builder.clone(),
            )
            .await?;
            Self::ensure_endpoint_identity_unchanged(&config, endpoint_identity.as_ref()).await?;

            // Setup genesis tx_env and evm_evm.
            let deployer = Address::with_last_byte(0x1);
            let mut tx_env = TxEnvFor::<FEN>::default();
            tx_env.set_caller(deployer);
            tx_env.set_kind(TxKind::Create);
            tx_env.set_data(Bytes::from(local_bytecode_vec));
            tx_env.set_chain_id(Some(evm_env.cfg_env.chain_id));
            tx_env.set_gas_limit(evm_env.block_env.gas_limit());
            tx_env.set_gas_price(evm_env.block_env.basefee() as u128);

            let kind = TxKind::Create;
            let block_context =
                if !maybe_predeploy && deploy_block != 0 && config.networks.is_monad() {
                    Some(monad_block_context::<FEN>(&config, deploy_block).await?)
                } else {
                    None
                };
            let target_context =
                synthetic_deployment_context::<FEN>(block_context.as_ref(), &tx_env);

            // Seed deployer account with funds
            let account_info = AccountInfo {
                balance: U256::from(100 * 10_u128.pow(18)),
                nonce: 0,
                ..Default::default()
            };
            executor.backend_mut().insert_account_info(deployer, account_info);

            let fork_address = crate::utils::deploy_contract::<FEN>(
                &mut executor,
                &evm_env,
                &tx_env,
                kind,
                target_context,
            )?;

            // Compare runtime bytecode. The onchain code is read at `deploy_block` to stay
            // anchored to the same height as the local fork. Predeploys keep reading at the
            // latest block: their code is stable and genesis state often isn't served by RPCs.
            let (deployed_bytecode, onchain_runtime_code) = crate::utils::get_runtime_codes::<FEN>(
                &mut executor,
                &provider,
                self.address,
                fork_address,
                (!maybe_predeploy).then_some(deploy_block),
            )
            .await?;
            Self::ensure_endpoint_identity_unchanged(&config, endpoint_identity.as_ref()).await?;

            let match_type = crate::utils::match_bytecodes(
                deployed_bytecode.original_byte_slice(),
                &onchain_runtime_code,
                &constructor_args,
                true,
                config.bytecode_hash,
            );

            crate::utils::print_result(
                match_type,
                BytecodeType::Runtime,
                &mut json_results,
                etherscan_metadata,
                &config,
            );

            if shell::is_json() {
                sh_println!("{}", serde_json::to_string(&json_results)?)?;
            }

            return Ok(());
        }

        // We can unwrap directly as maybe_predeploy is false
        let creation_data = creation_data.unwrap();
        // Get transaction and receipt.
        trace!(creation_tx_hash = ?creation_data.transaction_hash);
        let transaction = provider
            .get_transaction_by_hash(creation_data.transaction_hash)
            .await
            .or_else(|e| {
                eyre::bail!("Couldn't fetch transaction from RPC: {:?}", e);
            })?
            .ok_or_else(|| {
                eyre::eyre!("Transaction not found for hash {}", creation_data.transaction_hash)
            })?;
        let tx_hash = transaction.tx_hash();
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash)
            .await
            .or_else(|e| {
                eyre::bail!("Couldn't fetch transaction receipt from RPC: {:?}", e);
            })?;
        let receipt = if let Some(receipt) = receipt {
            receipt
        } else {
            eyre::bail!(
                "Receipt not found for transaction hash {}",
                creation_data.transaction_hash
            );
        };

        let creation_block = transaction.block_number();

        // Extract creation code from creation tx input.
        let maybe_creation_code = if receipt.to().is_none()
            && receipt.contract_address() == Some(self.address)
        {
            transaction.input().clone()
        } else if receipt.to() == Some(DEFAULT_CREATE2_DEPLOYER) {
            Bytes::copy_from_slice(&transaction.input()[32..])
        } else {
            // Try to get creation bytecode from tx trace.
            let traces = provider
                .trace_transaction(creation_data.transaction_hash)
                .await
                .unwrap_or_default();

            let creation_bytecode =
                traces.iter().find_map(|trace| match (&trace.trace.result, &trace.trace.action) {
                    (
                        Some(TraceOutput::Create(CreateOutput { address, .. })),
                        Action::Create(CreateAction { init, .. }),
                    ) if *address == self.address => Some(init.clone()),
                    _ => None,
                });

            creation_bytecode.ok_or_else(|| {
                eyre::eyre!(
                    "Could not extract the creation code for contract at address {}",
                    self.address
                )
            })?
        };
        Self::ensure_endpoint_identity_unchanged(&config, endpoint_identity.as_ref()).await?;

        // In some cases, Etherscan will return incorrect constructor arguments. If this
        // happens, try extracting arguments ourselves. Never replace user-provided arguments.
        if !args_from_user && !maybe_creation_code.ends_with(&constructor_args) {
            trace!("mismatch of constructor args with etherscan");
            if maybe_creation_code.len() >= local_bytecode.len() {
                // If local bytecode is longer than on-chain one, this is probably not a match.
                constructor_args =
                    Bytes::copy_from_slice(&maybe_creation_code[local_bytecode.len()..]);
                trace!(
                    target: "forge::verify",
                    "setting constructor args to latest {} bytes of bytecode",
                    constructor_args.len()
                );
            }
        }

        // Append constructor args to the local_bytecode.
        trace!(%constructor_args);
        let mut local_bytecode_vec = local_bytecode.to_vec();
        local_bytecode_vec.extend_from_slice(&constructor_args);

        // A suffix check alone is insufficient for dynamic ABI values: one valid encoding can
        // be a suffix of a different valid encoding. Always compare the complete creation code
        // when arguments came from the user, even if creation output is ignored.
        let creation_match_type = crate::utils::match_bytecodes(
            local_bytecode_vec.as_slice(),
            &maybe_creation_code,
            &constructor_args,
            false,
            config.bytecode_hash,
        );
        if args_from_user
            && creation_match_type.is_none()
            && self.ignore.is_none_or(|b| !b.is_creation())
        {
            let message =
                "Provided constructor args could not be validated against deployment creation code";
            if shell::is_json() {
                json_results.push(JsonResult {
                    bytecode_type: BytecodeType::Creation,
                    match_type: None,
                    message: Some(message.to_string()),
                });
                if self.ignore.is_none_or(|b| !b.is_runtime()) {
                    json_results.push(JsonResult {
                        bytecode_type: BytecodeType::Runtime,
                        match_type: None,
                        message: Some(message.to_string()),
                    });
                }
                sh_println!("{}", serde_json::to_string(&json_results)?)?;
            } else {
                sh_warn!("{message}")?;
                crate::utils::print_result(
                    None,
                    BytecodeType::Creation,
                    &mut json_results,
                    etherscan_metadata,
                    &config,
                );
                if self.ignore.is_none_or(|b| !b.is_runtime()) {
                    crate::utils::print_result(
                        None,
                        BytecodeType::Runtime,
                        &mut json_results,
                        etherscan_metadata,
                        &config,
                    );
                }
            }
            return Ok(());
        }

        trace!(ignore = ?self.ignore);
        // Check if `--ignore` is set to `creation`.
        if self.ignore.is_none_or(|b| !b.is_creation()) {
            // Compare creation code with locally built bytecode and `maybe_creation_code`.
            crate::utils::print_result(
                creation_match_type,
                BytecodeType::Creation,
                &mut json_results,
                etherscan_metadata,
                &config,
            );

            // If the creation code does not match, the runtime also won't match. Hence return.
            if creation_match_type.is_none() {
                crate::utils::print_result(
                    None,
                    BytecodeType::Runtime,
                    &mut json_results,
                    etherscan_metadata,
                    &config,
                );
                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&json_results)?)?;
                }
                return Ok(());
            }
        }

        if self.ignore.is_none_or(|b| !b.is_runtime()) {
            // Runtime verification can only re-deploy local bytecode for direct `CREATE` and the
            // default `CREATE2` deployer, so skip custom factory deployments.
            if let TxKind::Call(to) = ConsensusTransaction::kind(&transaction)
                && to != DEFAULT_CREATE2_DEPLOYER
            {
                let message = format!(
                    "Runtime bytecode verification is not supported for this contract: its \
                     creation transaction calls custom factory {to}. forge can only verify \
                     runtime bytecode for direct CREATE transactions and calls to the default \
                     CREATE2 deployer; skipping runtime bytecode verification."
                );
                if shell::is_json() {
                    json_results.push(JsonResult {
                        bytecode_type: BytecodeType::Runtime,
                        match_type: None,
                        message: Some(message),
                    });
                    sh_println!("{}", serde_json::to_string(&json_results)?)?;
                } else {
                    sh_warn!("{message}")?;
                }
                return Ok(());
            }

            // Get contract creation block.
            let simulation_block = match self.block {
                Some(BlockId::Number(BlockNumberOrTag::Number(block))) => block,
                Some(_) => {
                    eyre::bail!("Invalid block number");
                }
                None => creation_block.ok_or_else(|| {
                    eyre::eyre!(
                        "Failed to get block number of the contract creation tx, specify using the \
                         --block flag"
                    )
                })?,
            };

            // Fork the chain immediately before `simulation_block`, then execute with the target
            // block's environment and effective runtime hardfork.
            let block = provider.get_block(simulation_block.into()).full().await?;
            let (mut fork_config, mut evm_opts) = load_fork_config_and_evm_opts(&config)?;
            Self::apply_endpoint_expectation(
                &mut evm_opts,
                endpoint_identity.as_ref(),
                network_was_inferred,
            );
            let (mut evm_env, _tx_env, mut executor) = crate::utils::get_tracing_executor::<FEN>(
                &mut fork_config,
                simulation_block - 1, // env.fork_block_number
                simulation_block,
                block.as_ref(),
                evm_opts,
                executor_builder,
            )
            .await?;
            Self::ensure_endpoint_identity_unchanged(&config, endpoint_identity.as_ref()).await?;

            // Workaround for the NonceTooHigh issue as we're not simulating prior txs of the same
            // block.
            let prev_block_id = BlockId::number(simulation_block - 1);

            // Use `transaction.from` instead of `creation_data.contract_creator` to resolve
            // blockscout creation data discrepancy in case of CREATE2.
            let prev_block_nonce =
                provider.get_transaction_count(transaction.from()).block_id(prev_block_id).await?;

            apply_chain_specific_tx_replay_env_changes_for_chain(&mut evm_env, chain.id());
            let target_context = replay_block(
                &config,
                block.as_ref(),
                simulation_block,
                tx_hash,
                &mut executor,
                &evm_env,
            )
            .await?;

            let kind = ConsensusTransaction::kind(&transaction);
            let mut tx_env = TxEnvFor::<FEN>::from_any_rpc_transaction(&transaction)?;
            tx_env.set_nonce(prev_block_nonce);
            let target_context =
                target_context.unwrap_or_else(|| ChainFor::<FEN>::for_transaction(&tx_env));

            // Replace the `input` with local creation code in the creation tx.
            if let TxKind::Call(to) = kind {
                if to == DEFAULT_CREATE2_DEPLOYER {
                    let mut input = transaction.input()[..32].to_vec(); // Salt
                    input.extend_from_slice(&local_bytecode_vec);
                    tx_env.set_data(Bytes::from(input));

                    // Deploy default CREATE2 deployer
                    executor.deploy_create2_deployer()?;
                }
            } else {
                tx_env.set_data(Bytes::from(local_bytecode_vec));
            }

            let fork_address = crate::utils::deploy_contract::<FEN>(
                &mut executor,
                &evm_env,
                &tx_env,
                kind,
                target_context,
            )?;

            // State committed using deploy_with_env, now get the runtime bytecode from the db.
            let (fork_runtime_code, onchain_runtime_code) = crate::utils::get_runtime_codes::<FEN>(
                &mut executor,
                &provider,
                self.address,
                fork_address,
                Some(simulation_block),
            )
            .await?;
            Self::ensure_endpoint_identity_unchanged(&config, endpoint_identity.as_ref()).await?;

            // Compare the onchain runtime bytecode with the runtime code from the fork.
            let match_type = crate::utils::match_bytecodes(
                fork_runtime_code.original_byte_slice(),
                &onchain_runtime_code,
                &constructor_args,
                true,
                config.bytecode_hash,
            );

            crate::utils::print_result(
                match_type,
                BytecodeType::Runtime,
                &mut json_results,
                etherscan_metadata,
                &config,
            );
        }

        if shell::is_json() {
            sh_println!("{}", serde_json::to_string(&json_results)?)?;
        }
        Ok(())
    }
}

type ReplayBlockFuture<'a, FEN> = Pin<Box<dyn Future<Output = Result<Option<ChainFor<FEN>>>> + 'a>>;
type ReplayBlockFn<FEN> = for<'a> fn(
    &'a Config,
    Option<&'a AnyRpcBlock>,
    u64,
    B256,
    &'a mut TracingExecutor<FEN>,
    &'a EvmEnvFor<FEN>,
) -> ReplayBlockFuture<'a, FEN>;

/// Replays ordinary transactions preceding `target_hash` and returns its execution context.
fn replay_block_transactions<'a, FEN: FoundryEvmNetwork>(
    _config: &'a Config,
    block: Option<&'a AnyRpcBlock>,
    _block_number: u64,
    target_hash: B256,
    executor: &'a mut TracingExecutor<FEN>,
    evm_env: &'a EvmEnvFor<FEN>,
) -> ReplayBlockFuture<'a, FEN> {
    Box::pin(async move {
        let Some(block) = block else { return Ok(None) };
        let BlockTransactions::Full(txs) = block.transactions() else {
            return Err(eyre::eyre!("Could not get block txs"));
        };
        let target_tx = txs
            .iter()
            .find(|tx| tx.tx_hash() == target_hash)
            .ok_or_else(|| eyre::eyre!("transaction {target_hash:?} is missing from its block"))?;
        let target_tx_env = TxEnvFor::<FEN>::from_any_rpc_transaction(target_tx)?;

        for tx in txs {
            trace!("replay tx::: {}", tx.tx_hash());
            if tx.tx_hash() == target_hash {
                break;
            }
            if is_known_system_sender(tx.from())
                || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE)
            {
                continue;
            }

            let tx_env = TxEnvFor::<FEN>::from_any_rpc_transaction(tx)?;
            let chain_context = ChainFor::<FEN>::for_transaction(&tx_env);
            execute_replay_transaction(executor, evm_env, tx, tx_env, chain_context)?;
        }

        Ok(Some(ChainFor::<FEN>::for_transaction(&target_tx_env)))
    })
}

/// Replays Monad transactions preceding `target_hash` with their ancestry context.
#[cfg(feature = "monad")]
fn replay_monad_block_transactions<'a>(
    config: &'a Config,
    block: Option<&'a AnyRpcBlock>,
    block_number: u64,
    target_hash: B256,
    executor: &'a mut TracingExecutor<MonadEvmNetwork>,
    evm_env: &'a EvmEnvFor<MonadEvmNetwork>,
) -> ReplayBlockFuture<'a, MonadEvmNetwork> {
    Box::pin(async move {
        let block = block.ok_or_else(|| {
            eyre::eyre!("block {block_number} is required to reconstruct transaction context")
        })?;
        let BlockTransactions::Full(txs) = block.transactions() else {
            return Err(eyre::eyre!("Could not get block txs"));
        };
        let block_context = monad_block_context::<MonadEvmNetwork>(config, block_number).await?;
        let target_index = txs
            .iter()
            .position(|tx| tx.tx_hash() == target_hash)
            .ok_or_else(|| eyre::eyre!("transaction {target_hash:?} is missing from its block"))?;

        for (index, tx) in txs.iter().enumerate() {
            trace!("replay tx::: {}", tx.tx_hash());
            if tx.tx_hash() == target_hash {
                break;
            }

            let tx_env = TxEnvFor::<MonadEvmNetwork>::from_any_rpc_transaction(tx)?;
            let chain_context = block_context.transaction(index);
            if is_known_system_sender(tx.from())
                || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE)
            {
                let _ = executor
                    .try_transact_system_replay_with_env_and_context(
                        evm_env.clone(),
                        tx_env,
                        chain_context,
                    )
                    .wrap_err_with(|| {
                        format!(
                            "Failed to replay system transaction: {:?} in block {}",
                            tx.tx_hash(),
                            evm_env.block_env.number()
                        )
                    })?;
                continue;
            }

            execute_replay_transaction(executor, evm_env, tx, tx_env, chain_context)?;
        }

        Ok(Some(block_context.transaction(target_index)))
    })
}

fn execute_replay_transaction<FEN: FoundryEvmNetwork>(
    executor: &mut TracingExecutor<FEN>,
    evm_env: &EvmEnvFor<FEN>,
    tx: &alloy_network::AnyRpcTransaction,
    tx_env: TxEnvFor<FEN>,
    chain_context: ChainFor<FEN>,
) -> Result<()> {
    if ConsensusTransaction::to(tx).is_some() {
        executor
            .transact_with_env_and_context(evm_env.clone(), tx_env, chain_context)
            .wrap_err_with(|| {
                format!(
                    "Failed to execute transaction: {:?} in block {}",
                    tx.tx_hash(),
                    evm_env.block_env.number()
                )
            })?;
    } else if let Err(error) =
        executor.deploy_with_env_and_context(evm_env.clone(), tx_env, chain_context, None)
    {
        match error {
            // Reverted transactions should be skipped.
            EvmError::Execution(_) => (),
            error => {
                return Err(error).wrap_err_with(|| {
                    format!(
                        "Failed to deploy transaction: {:?} in block {}",
                        tx.tx_hash(),
                        evm_env.block_env.number()
                    )
                });
            }
        }
    }
    Ok(())
}

/// Fetches the block context Monad needs to reconstruct replay ordering.
async fn monad_block_context<FEN: FoundryEvmNetwork>(
    config: &Config,
    block_number: u64,
) -> Result<BlockContext<FEN>> {
    let provider = ProviderBuilder::<FEN::Network>::from_config(config)?.build()?;
    let block = provider.get_block(block_number.into()).full().await?.ok_or_else(|| {
        eyre::eyre!("block {block_number} is required to reconstruct transaction context")
    })?;
    BlockContext::<FEN>::fetch(&provider, &block).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_tempo_network() {
        let args = VerifyBytecodeArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter.sol:Counter",
            "--network",
            "tempo",
        ]);

        assert_eq!(args.network, Some(NetworkVariant::Tempo));
    }

    #[test]
    #[cfg(feature = "monad")]
    fn can_parse_monad_network() {
        let args = VerifyBytecodeArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter.sol:Counter",
            "--network",
            "monad",
        ]);

        assert_eq!(args.network, Some(NetworkVariant::Monad));
    }

    #[test]
    fn configured_network_uses_tempo_config_network() {
        let config = Config { networks: NetworkVariant::Tempo.into(), ..Default::default() };

        assert_eq!(
            VerifyBytecodeArgs::configured_network(None, &config),
            Some(NetworkVariant::Tempo)
        );
    }

    #[test]
    fn configured_network_preserves_celo_execution_profile() {
        let mut config = Config {
            networks: foundry_evm_networks::NetworkConfigs::with_celo(),
            ..Default::default()
        };
        let endpoint_identity = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Tempo,
            network_profile: NetworkVariant::Tempo.into(),
            reported_hardfork: None,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };

        assert_eq!(
            VerifyBytecodeArgs::configured_network(None, &config),
            Some(NetworkVariant::Ethereum)
        );
        assert_eq!(
            VerifyBytecodeArgs::materialize_execution_network(
                &mut config,
                Some(&endpoint_identity)
            ),
            NetworkVariant::Ethereum
        );
        assert!(config.networks.is_celo());
    }

    #[test]
    fn verify_bytecode_requires_stable_endpoint_identity() {
        let expected = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: Default::default(),
            reported_hardfork: Some("FutureA".to_string()),
            hardfork: None,
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };

        assert!(VerifyBytecodeArgs::validate_endpoint_identity(&expected, &expected).is_ok());

        let mut reset = expected.clone();
        reset.instance_id = Some(B256::with_last_byte(2));
        assert!(VerifyBytecodeArgs::validate_endpoint_identity(&expected, &reset).is_err());

        let mut changed_hardfork = expected.clone();
        changed_hardfork.reported_hardfork = Some("FutureB".to_string());
        assert!(
            VerifyBytecodeArgs::validate_endpoint_identity(&expected, &changed_hardfork).is_err()
        );

        let mut evm_opts = EvmOpts::default();
        VerifyBytecodeArgs::apply_endpoint_expectation(&mut evm_opts, Some(&expected), true);
        assert_eq!(evm_opts.expected_fork_endpoint, Some(expected));
        assert!(evm_opts.fork_network_is_inferred);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn configured_network_uses_monad_config_network() {
        let config = Config { networks: NetworkVariant::Monad.into(), ..Default::default() };

        assert_eq!(
            VerifyBytecodeArgs::configured_network(None, &config),
            Some(NetworkVariant::Monad)
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn configured_network_prefers_cli_network() {
        let config = Config { networks: NetworkVariant::Monad.into(), ..Default::default() };

        assert_eq!(
            VerifyBytecodeArgs::configured_network(Some(NetworkVariant::Ethereum), &config),
            Some(NetworkVariant::Ethereum)
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn nested_endpoint_separates_execution_family_from_explorer_chain() {
        let identity = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 1,
            source_chain_id: 143,
            network: NetworkVariant::Monad,
            network_profile: NetworkVariant::Monad.into(),
            reported_hardfork: None,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: Some(123),
            source_fork_block_hash: None,
        };

        assert_eq!(
            VerifyBytecodeArgs::effective_network(None, Some(&identity)),
            NetworkVariant::Monad
        );
        assert_eq!(VerifyBytecodeArgs::explorer_chain(None, Some(&identity)).unwrap().id(), 143);
        assert_eq!(
            VerifyBytecodeArgs::explorer_chain(Some(Chain::from_id(1)), Some(&identity))
                .unwrap()
                .id(),
            1
        );
    }

    #[cfg(feature = "base")]
    #[test]
    fn configured_network_preserves_base() {
        let config = Config { networks: NetworkVariant::Base.into(), ..Default::default() };
        assert_eq!(
            VerifyBytecodeArgs::configured_network(None, &config),
            Some(NetworkVariant::Base)
        );
    }
}
