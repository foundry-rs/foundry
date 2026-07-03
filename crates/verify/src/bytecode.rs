//! The `forge verify-bytecode` command.
use crate::{
    etherscan::EtherscanVerificationProvider,
    utils::{
        BytecodeType, JsonResult, check_and_encode_args, check_explorer_args, configure_env_block,
        load_fork_config_and_evm_opts, maybe_predeploy_contract,
    },
    verify::VerifierArgs,
};
use alloy_consensus::{BlockHeader, Transaction as ConsensusTransaction};
use alloy_evm::FromRecoveredTx;
use alloy_primitives::{Address, Bytes, TxKind, U256, hex};
use alloy_provider::{
    Provider,
    ext::TraceApi,
    network::{
        AnyNetwork, BlockResponse, ReceiptResponse, TransactionResponse,
        primitives::BlockTransactions,
    },
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
use foundry_config::{Config, figment, impl_figment_convert};
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER,
    core::{
        FoundryBlock as _, FoundryTransaction as _,
        evm::{EthEvmNetwork, FoundryEvmNetwork, SpecFor, TempoEvmNetwork, TxEnvFor},
    },
    executors::EvmError,
};
use foundry_evm_networks::NetworkVariant;
use revm::{context::Block as _, state::AccountInfo};
use std::path::PathBuf;

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
        cli_network.or_else(|| config.networks.resolved_network())
    }

    /// Run the `verify-bytecode` command to verify the bytecode onchain against the locally built
    /// bytecode.
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;
        let network = if let Some(network) = Self::configured_network(self.network, &config) {
            if self.network.is_some() {
                config.networks = network.into();
            }
            network
        } else {
            let network = {
                let provider = ProviderBuilder::<AnyNetwork>::from_config(&config)?.build()?;
                provider.get_chain_id().await?.into()
            };

            if !matches!(network, NetworkVariant::Ethereum) {
                config.networks = network.into();
            }

            network
        };

        match network {
            NetworkVariant::Ethereum => {
                self.run_with_network_and_config::<EthEvmNetwork>(config).await
            }
            #[cfg(feature = "optimism")]
            NetworkVariant::Optimism => {
                self.run_with_network_and_config::<OpEvmNetwork>(config).await
            }
            NetworkVariant::Tempo => {
                self.run_with_network_and_config::<TempoEvmNetwork>(config).await
            }
        }
    }

    async fn run_with_network_and_config<FEN>(mut self, config: Config) -> Result<()>
    where
        FEN: FoundryEvmNetwork,
    {
        // Setup
        let provider = ProviderBuilder::<FEN::Network>::from_config(&config)?.build()?;

        // If chain is not set, we try to get it from the RPC.
        // If RPC is not set, the default chain is used.
        let chain = match config.get_rpc_url() {
            Some(_) => utils::get_chain::<FEN::Network, _>(config.chain, &provider).await?,
            None => config.chain.unwrap_or_default(),
        };

        // Set Etherscan options.
        self.etherscan.chain = Some(chain);
        self.etherscan.key = config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);

        // Whether the user explicitly configured a block explorer. Client setup errors are only
        // treated as "no explorer available" when nothing was configured: an explicitly provided
        // verifier or API key that fails to resolve must still surface as an error.
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

        // The EVM version to verify against: the explorer-reported version when available,
        // otherwise the local project configuration.
        let evm_version = match etherscan_metadata {
            Some(metadata) => metadata.evm_version()?.unwrap_or_default(),
            None => config.evm_version,
        };

        // Obtain local artifact
        let artifact = crate::utils::build_project(&self, &config)?;

        // Get local bytecode (creation code)
        let local_bytecode = artifact
            .bytecode
            .as_ref()
            .and_then(|b| b.to_owned().into_bytes())
            .ok_or_eyre("Unlinked bytecode is not supported for verification")?;

        // Get and encode user provided constructor args
        let provided_constructor_args = if let Some(path) = self.constructor_args_path.clone() {
            // Read from file
            Some(read_constructor_args_file(path)?)
        } else {
            self.constructor_args.clone()
        }
        .map(|args| check_and_encode_args(&artifact, args))
        .transpose()?
        .or(self.encoded_constructor_args.clone().map(hex::decode).transpose()?);

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
                    Some(_) => eyre::bail!("Invalid block number"),
                    None => provider.get_block_number().await?,
                }
            };

            // Append constructor args to the local_bytecode.
            trace!(%constructor_args);
            let mut local_bytecode_vec = local_bytecode.to_vec();
            local_bytecode_vec.extend_from_slice(&constructor_args);

            let (mut fork_config, evm_opts) = load_fork_config_and_evm_opts(&config)?;
            let (mut evm_env, _, mut executor) = crate::utils::get_tracing_executor::<FEN>(
                &mut fork_config,
                deploy_block,
                evm_version,
                evm_opts,
            )
            .await?;

            evm_env.block_env.set_number(U256::from(deploy_block));
            let deploy_block_info = provider.get_block(deploy_block.into()).full().await?;

            // Setup genesis tx_env and evm_evm.
            let deployer = Address::with_last_byte(0x1);
            let mut tx_env = TxEnvFor::<FEN>::default();
            tx_env.set_caller(deployer);
            tx_env.set_kind(TxKind::Create);
            tx_env.set_data(Bytes::from(local_bytecode_vec));
            tx_env.set_chain_id(Some(evm_env.cfg_env.chain_id));
            tx_env.set_gas_limit(evm_env.block_env.gas_limit());
            tx_env.set_gas_price(evm_env.block_env.basefee() as u128);

            if let Some(ref block) = deploy_block_info {
                configure_env_block::<FEN>(&mut evm_env, block, config.networks);
                tx_env.set_gas_limit(block.header().gas_limit());
                tx_env.set_gas_price(block.header().base_fee_per_gas().unwrap_or_default() as u128);
            }

            let kind = TxKind::Create;

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
                config.evm_spec_id::<SpecFor<FEN>>(),
                kind,
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
            .or_else(|e| eyre::bail!("Couldn't fetch transaction from RPC: {:?}", e))?
            .ok_or_else(|| {
                eyre::eyre!("Transaction not found for hash {}", creation_data.transaction_hash)
            })?;
        let tx_hash = transaction.tx_hash();
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

        // In some cases, Etherscan will return incorrect constructor arguments. If this
        // happens, try extracting arguments ourselves.
        if !maybe_creation_code.ends_with(&constructor_args) {
            trace!("mismatch of constructor args with etherscan");
            // If local bytecode is longer than on-chain one, this is probably not a match.
            if maybe_creation_code.len() >= local_bytecode.len() {
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

        trace!(ignore = ?self.ignore);
        // Check if `--ignore` is set to `creation`.
        if self.ignore.is_none_or(|b| !b.is_creation()) {
            // Compare creation code with locally built bytecode and `maybe_creation_code`.
            let match_type = crate::utils::match_bytecodes(
                local_bytecode_vec.as_slice(),
                &maybe_creation_code,
                &constructor_args,
                false,
                config.bytecode_hash,
            );

            crate::utils::print_result(
                match_type,
                BytecodeType::Creation,
                &mut json_results,
                etherscan_metadata,
                &config,
            );

            // If the creation code does not match, the runtime also won't match. Hence return.
            if match_type.is_none() {
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
            // Get contract creation block.
            let simulation_block = match self.block {
                Some(BlockId::Number(BlockNumberOrTag::Number(block))) => block,
                Some(_) => eyre::bail!("Invalid block number"),
                None => {
                    creation_block.ok_or_else(|| {
                        eyre::eyre!("Failed to get block number of the contract creation tx, specify using the --block flag")
                    })?
                }
            };

            // Fork the chain at `simulation_block`.
            let (mut fork_config, evm_opts) = load_fork_config_and_evm_opts(&config)?;
            let (mut evm_env, _tx_env, mut executor) = crate::utils::get_tracing_executor::<FEN>(
                &mut fork_config,
                simulation_block - 1, // env.fork_block_number
                evm_version,
                evm_opts,
            )
            .await?;
            evm_env.block_env.set_number(U256::from(simulation_block));
            let block = provider.get_block(simulation_block.into()).full().await?;

            // Workaround for the NonceTooHigh issue as we're not simulating prior txs of the same
            // block.
            let prev_block_id = BlockId::number(simulation_block - 1);

            // Use `transaction.from` instead of `creation_data.contract_creator` to resolve
            // blockscout creation data discrepancy in case of CREATE2.
            let prev_block_nonce =
                provider.get_transaction_count(transaction.from()).block_id(prev_block_id).await?;

            if let Some(ref block) = block {
                configure_env_block::<FEN>(&mut evm_env, block, config.networks);

                let BlockTransactions::Full(txs) = block.transactions() else {
                    return Err(eyre::eyre!("Could not get block txs"));
                };

                // Replay txes in block until the contract creation one.
                for tx in txs {
                    trace!("replay tx::: {}", tx.tx_hash());
                    if is_known_system_sender(tx.from())
                        || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE)
                    {
                        continue;
                    }
                    if tx.tx_hash() == tx_hash {
                        break;
                    }

                    let tx_env = TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from());

                    if ConsensusTransaction::to(tx).is_some() {
                        executor.transact_with_env(evm_env.clone(), tx_env.clone()).wrap_err_with(
                            || {
                                format!(
                                    "Failed to execute transaction: {:?} in block {}",
                                    tx.tx_hash(),
                                    evm_env.block_env.number()
                                )
                            },
                        )?;
                    } else if let Err(error) =
                        executor.deploy_with_env(evm_env.clone(), tx_env.clone(), None)
                    {
                        match error {
                            // Reverted transactions should be skipped
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
                }
            }

            let kind = ConsensusTransaction::kind(&transaction);
            let mut tx_env =
                TxEnvFor::<FEN>::from_recovered_tx(transaction.as_ref(), transaction.from());
            tx_env.set_nonce(prev_block_nonce);

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
                config.evm_spec_id::<SpecFor<FEN>>(),
                kind,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_network() {
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
    fn configured_network_uses_config_network() {
        let config = Config { networks: NetworkVariant::Tempo.into(), ..Default::default() };

        assert_eq!(
            VerifyBytecodeArgs::configured_network(None, &config),
            Some(NetworkVariant::Tempo)
        );
    }

    #[test]
    fn configured_network_prefers_cli_network() {
        let config = Config { networks: NetworkVariant::Tempo.into(), ..Default::default() };

        assert_eq!(
            VerifyBytecodeArgs::configured_network(Some(NetworkVariant::Ethereum), &config),
            Some(NetworkVariant::Ethereum)
        );
    }
}
