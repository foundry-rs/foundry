//! The `forge verify-bytecode` command.
use crate::{
    etherscan::EtherscanVerificationProvider,
    utils::{
        check_and_encode_args, check_explorer_args, configure_env_block, maybe_predeploy_contract,
        BytecodeType, JsonResult,
    },
    verify::VerifierArgs,
};
use alloy_primitives::{hex, Address, Bytes, U256};
use alloy_provider::{
    network::{AnyTxEnvelope, TransactionBuilder},
    Provider,
};
use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionInput, TransactionRequest};
use clap::{Parser, ValueHint};
use eyre::{Context, OptionExt, Result};
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, read_constructor_args_file, LoadConfig},
};
use foundry_common::shell;
use foundry_compilers::{artifacts::EvmVersion, info::ContractInfo};
use foundry_config::{figment, impl_figment_convert, Config};
use foundry_evm::{constants::DEFAULT_CREATE2_DEPLOYER, utils::configure_tx_req_env};
use revm_primitives::{AccountInfo, TxKind};
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
        let provider = utils::get_provider(&config)?;

        // If chain is not set, we try to get it from the RPC.
        // If RPC is not set, the default chain is used.
        let chain = match config.get_rpc_url() {
            Some(_) => utils::get_chain(config.chain, &provider).await?,
            None => config.chain.unwrap_or_default(),
        };

        // Set Etherscan options.
        self.etherscan.chain = Some(chain);
        self.etherscan.key = config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);

        // Etherscan client
        let etherscan = EtherscanVerificationProvider.client(
            self.etherscan.chain.unwrap_or_default(),
            self.verifier.verifier_url.as_deref(),
            self.etherscan.key().as_deref(),
            &config,
        )?;

        // Get the bytecode at the address, bailing if it doesn't exist.
        let code = provider.get_code_at(self.address).await?;
        if code.is_empty() {
            eyre::bail!("No bytecode found at address {}", self.address);
        }

        if !shell::is_json() {
            sh_println!(
                "Verifying bytecode for contract {} at address {}",
                self.contract.name,
                self.address
            )?;
        }

        let mut json_results: Vec<JsonResult> = vec![];

        // Get creation tx hash.
        let creation_data = etherscan.contract_creation_data(self.address).await;

        // Check if contract is a predeploy
        let (creation_data, maybe_predeploy) = maybe_predeploy_contract(creation_data)?;

        trace!(maybe_predeploy = ?maybe_predeploy);

        // Get the constructor args using `source_code` endpoint.
        let source_code = etherscan.contract_source_code(self.address).await?;

        // Check if the contract name matches.
        let name = source_code.items.first().map(|item| item.contract_name.to_owned());
        if name.as_ref() != Some(&self.contract.name) {
            eyre::bail!("Contract name mismatch");
        }

        // Obtain Etherscan compilation metadata.
        let etherscan_metadata = source_code.items.first().unwrap();

        // Obtain local artifact
        let artifact = if let Ok(local_bytecode) =
            crate::utils::build_using_cache(&self, etherscan_metadata, &config)
        {
            trace!("using cache");
            local_bytecode
        } else {
            crate::utils::build_project(&self, &config)?
        };

        // Get local bytecode (creation code)
        let local_bytecode = artifact
            .bytecode
            .as_ref()
            .and_then(|b| b.to_owned().into_bytes())
            .ok_or_eyre("Unlinked bytecode is not supported for verification")?;

        // Get and encode user provided constructor args
        let provided_constructor_args = if let Some(path) = self.constructor_args_path.to_owned() {
            // Read from file
            Some(read_constructor_args_file(path)?)
        } else {
            self.constructor_args.to_owned()
        }
        .map(|args| check_and_encode_args(&artifact, args))
        .transpose()?
        .or(self.encoded_constructor_args.to_owned().map(hex::decode).transpose()?);

        let mut constructor_args = if let Some(provided) = provided_constructor_args {
            provided.into()
        } else {
            // If no constructor args were provided, try to retrieve them from the explorer.
            check_explorer_args(source_code.clone())?
        };

        // This fails only when the contract expects constructor args but NONE were provided OR
        // retrieved from explorer (in case of predeploys).
        crate::utils::check_args_len(&artifact, &constructor_args)?;

        if maybe_predeploy {
            if !shell::is_json() {
                sh_warn!(
                    "Attempting to verify predeployed contract at {:?}. Ignoring creation code verification.",
                    self.address
                )?;
            }

            // Append constructor args to the local_bytecode.
            trace!(%constructor_args);
            let mut local_bytecode_vec = local_bytecode.to_vec();
            local_bytecode_vec.extend_from_slice(&constructor_args);

            // Deploy at genesis
            let gen_blk_num = 0_u64;
            let (mut fork_config, evm_opts) = config.clone().load_config_and_evm_opts()?;
            let (mut env, mut executor) = crate::utils::get_tracing_executor(
                &mut fork_config,
                gen_blk_num,
                etherscan_metadata.evm_version()?.unwrap_or(EvmVersion::default()),
                evm_opts,
            )
            .await?;

            env.block.number = U256::ZERO; // Genesis block
            let genesis_block = provider.get_block(gen_blk_num.into(), true.into()).await?;

            // Setup genesis tx and env.
            let deployer = Address::with_last_byte(0x1);
            let mut gen_tx_req = TransactionRequest::default()
                .with_from(deployer)
                .with_input(Bytes::from(local_bytecode_vec))
                .into_create();

            if let Some(ref block) = genesis_block {
                configure_env_block(&mut env, block);
                gen_tx_req.max_fee_per_gas = block.header.base_fee_per_gas.map(|g| g as u128);
                gen_tx_req.gas = Some(block.header.gas_limit);
                gen_tx_req.gas_price = block.header.base_fee_per_gas.map(|g| g as u128);
            }

            // configure_tx_rq_env(&mut env, &gen_tx);

            configure_tx_req_env(&mut env, &gen_tx_req, None)
                .wrap_err("Failed to configure tx request env")?;

            // Seed deployer account with funds
            let account_info = AccountInfo {
                balance: U256::from(100 * 10_u128.pow(18)),
                nonce: 0,
                ..Default::default()
            };
            executor.backend_mut().insert_account_info(deployer, account_info);

            let fork_address = crate::utils::deploy_contract(
                &mut executor,
                &env,
                config.evm_spec_id(),
                gen_tx_req.to,
            )?;

            // Compare runtime bytecode
            let (deployed_bytecode, onchain_runtime_code) = crate::utils::get_runtime_codes(
                &mut executor,
                &provider,
                self.address,
                fork_address,
                None,
            )
            .await?;

            let match_type = crate::utils::match_bytecodes(
                &deployed_bytecode.original_bytes(),
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

        let mut transaction: TransactionRequest = match transaction.inner.inner {
            AnyTxEnvelope::Ethereum(tx) => tx.into(),
            AnyTxEnvelope::Unknown(_) => unreachable!("Unknown transaction type"),
        };

        // Extract creation code from creation tx input.
        let maybe_creation_code =
            if receipt.to.is_none() && receipt.contract_address == Some(self.address) {
                match &transaction.input.input {
                    Some(input) => &input[..],
                    None => unreachable!("creation tx input is None"),
                }
            } else if receipt.to == Some(DEFAULT_CREATE2_DEPLOYER) {
                match &transaction.input.input {
                    Some(input) => &input[32..],
                    None => unreachable!("creation tx input is None"),
                }
            } else {
                eyre::bail!(
                    "Could not extract the creation code for contract at address {}",
                    self.address
                );
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
        if !self.ignore.is_some_and(|b| b.is_creation()) {
            // Compare creation code with locally built bytecode and `maybe_creation_code`.
            let match_type = crate::utils::match_bytecodes(
                local_bytecode_vec.as_slice(),
                maybe_creation_code,
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

        if !self.ignore.is_some_and(|b| b.is_runtime()) {
            // Get contract creation block.
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

            // Fork the chain at `simulation_block`.
            let (mut fork_config, evm_opts) = config.clone().load_config_and_evm_opts()?;
            let (mut env, mut executor) = crate::utils::get_tracing_executor(
                &mut fork_config,
                simulation_block - 1, // env.fork_block_number
                etherscan_metadata.evm_version()?.unwrap_or(EvmVersion::default()),
                evm_opts,
            )
            .await?;
            env.block.number = U256::from(simulation_block);
            let block = provider.get_block(simulation_block.into(), true.into()).await?;

            // Workaround for the NonceTooHigh issue as we're not simulating prior txs of the same
            // block.
            let prev_block_id = BlockId::number(simulation_block - 1);

            // Use `transaction.from` instead of `creation_data.contract_creator` to resolve
            // blockscout creation data discrepancy in case of CREATE2.
            let prev_block_nonce = provider
                .get_transaction_count(transaction.from.unwrap())
                .block_id(prev_block_id)
                .await?;
            transaction.set_nonce(prev_block_nonce);

            if let Some(ref block) = block {
                configure_env_block(&mut env, block)
            }

            // Replace the `input` with local creation code in the creation tx.
            if let Some(TxKind::Call(to)) = transaction.kind() {
                if to == DEFAULT_CREATE2_DEPLOYER {
                    let mut input = transaction.input.input.unwrap()[..32].to_vec(); // Salt
                    input.extend_from_slice(&local_bytecode_vec);
                    transaction.input = TransactionInput::both(Bytes::from(input));

                    // Deploy default CREATE2 deployer
                    executor.deploy_create2_deployer()?;
                }
            } else {
                transaction.input = TransactionInput::both(Bytes::from(local_bytecode_vec));
            }

            // configure_req__env(&mut env, &transaction.inner);
            configure_tx_req_env(&mut env, &transaction, None)
                .wrap_err("Failed to configure tx request env")?;

            let fork_address = crate::utils::deploy_contract(
                &mut executor,
                &env,
                config.evm_spec_id(),
                transaction.to,
            )?;

            // State committed using deploy_with_env, now get the runtime bytecode from the db.
            let (fork_runtime_code, onchain_runtime_code) = crate::utils::get_runtime_codes(
                &mut executor,
                &provider,
                self.address,
                fork_address,
                Some(simulation_block),
            )
            .await?;

            // Compare the onchain runtime bytecode with the runtime code from the fork.
            let match_type = crate::utils::match_bytecodes(
                &fork_runtime_code.original_bytes(),
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
