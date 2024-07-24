use super::{provider::VerificationProvider, verify::VerifyCheckArgs};
use crate::{
    bytecode::VerifyBytecodeArgs, provider::VerificationContext, retry::RETRY_CHECK_ON_VERIFY,
    types::VerificationType, verify::VerifyArgs,
};
use alloy_json_abi::Function;
use alloy_primitives::hex;
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use eyre::{eyre, Context, OptionExt, Result};
use foundry_block_explorers::{
    errors::EtherscanError,
    utils::lookup_compiler_version,
    verify::{CodeFormat, VerifyContract},
    Client,
};
use foundry_cli::utils::{get_provider, read_constructor_args_file, LoadConfig};
use foundry_common::{
    abi::encode_function_args,
    retry::{Retry, RetryError},
    shell,
};
use foundry_compilers::{
    artifacts::{BytecodeHash, BytecodeObject, EvmVersion},
    Artifact,
};
use foundry_config::{Chain, Config};
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER, executors::TracingExecutor, utils::configure_tx_env,
};
use futures::FutureExt;
use once_cell::sync::Lazy;
use regex::Regex;
use revm_primitives::{db::Database, Address, Bytes, EnvWithHandlerCfg, HandlerCfg, SpecId, U256};
use semver::{BuildMetadata, Version};
use std::fmt::Debug;
use yansi::Paint;

mod flatten;

mod prepare;
use prepare::{BytecodeType, JsonResult};

mod standard_json;

pub static RE_BUILD_COMMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?P<commit>commit\.[0-9,a-f]{8})").unwrap());

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct EtherscanVerificationProvider;

/// The contract source provider for [EtherscanVerificationProvider]
///
/// Returns source, contract_name and the source [CodeFormat]
trait EtherscanSourceProvider: Send + Sync + Debug {
    fn source(
        &self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<(String, String, CodeFormat)>;
}

#[async_trait::async_trait]
impl VerificationProvider for EtherscanVerificationProvider {
    async fn preflight_verify_check(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<()> {
        let _ = self.prepare_verify_request(&args, &context).await?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs, context: VerificationContext) -> Result<()> {
        let (etherscan, verify_args) = self.prepare_verify_request(&args, &context).await?;

        if !args.skip_is_verified_check &&
            self.is_contract_verified(&etherscan, &verify_args).await?
        {
            println!(
                "\nContract [{}] {:?} is already verified. Skipping verification.",
                verify_args.contract_name,
                verify_args.address.to_checksum(None)
            );

            return Ok(())
        }

        trace!(target: "forge::verify", ?verify_args, "submitting verification request");

        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| async {
                println!(
                    "\nSubmitting verification for [{}] {}.",
                    verify_args.contract_name, verify_args.address
                );
                let resp = etherscan
                    .submit_contract_verification(&verify_args)
                    .await
                    .wrap_err_with(|| {
                        // valid json
                        let args = serde_json::to_string(&verify_args).unwrap();
                        error!(target: "forge::verify", ?args, "Failed to submit verification");
                        format!("Failed to submit contract verification, payload:\n{args}")
                    })?;

                trace!(target: "forge::verify", ?resp, "Received verification response");

                if resp.status == "0" {
                    if resp.result == "Contract source code already verified"
                        // specific for blockscout response
                        || resp.result == "Smart-contract already verified."
                    {
                        return Ok(None)
                    }

                    if resp.result.starts_with("Unable to locate ContractCode at") {
                        warn!("{}", resp.result);
                        return Err(eyre!("Etherscan could not detect the deployment."))
                    }

                    warn!("Failed verify submission: {:?}", resp);
                    eprintln!(
                        "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
                        resp.message, resp.result
                    );
                    std::process::exit(1);
                }

                Ok(Some(resp))
            })
            .await?;

        if let Some(resp) = resp {
            println!(
                "Submitted contract for verification:\n\tResponse: `{}`\n\tGUID: `{}`\n\tURL: {}",
                resp.message,
                resp.result,
                etherscan.address_url(args.address)
            );

            if args.watch {
                let check_args = VerifyCheckArgs {
                    id: resp.result,
                    etherscan: args.etherscan,
                    retry: RETRY_CHECK_ON_VERIFY,
                    verifier: args.verifier,
                };
                // return check_args.run().await
                return self.check(check_args).await
            }
        } else {
            println!("Contract source code already verified");
        }

        Ok(())
    }

    async fn verify_bytecode(&mut self, args: VerifyBytecodeArgs) -> Result<()> {
        let (etherscan, config) = self.prepare_verify_bytecode_request(&args).await?;
        let provider = get_provider(&config)?;

        // Get the constructor args using `source_code` endpoint.
        let source_code = etherscan.contract_source_code(args.address).await?;

        // Check if the contract name matches.
        let name = source_code.items.first().map(|item| item.contract_name.to_owned());
        if name.as_ref() != Some(&args.contract.name) {
            eyre::bail!("Contract name mismatch");
        }

        // Get the constructor args from Etherscan.
        let constructor_args = if let Some(args) = source_code.items.first() {
            args.constructor_arguments.clone()
        } else {
            eyre::bail!("No constructor arguments found for contract at address {}", args.address);
        };

        // Get user provided constructor args.
        let provided_constructor_args = if let Some(args) = args.constructor_args.to_owned() {
            args
        } else if let Some(path) = args.constructor_args_path.to_owned() {
            // Read from file
            let res = read_constructor_args_file(path)?;
            // Convert res to Bytes
            res.join("")
        } else {
            constructor_args.to_string()
        };

        if provided_constructor_args != constructor_args.to_string() && !args.json {
            println!("{}", "The provided constructor args do not match the constructor args from Etherscan. This will result in a mismatch - Using the args from Etherscan".red().bold());
        }

        // Get creation tx hash.
        let creation_data = etherscan.contract_creation_data(args.address).await?;

        trace!(target: "forge::verify", creation_tx_hash = ?creation_data.transaction_hash);

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

        // Extract creation code.
        let maybe_creation_code =
            if receipt.to.is_none() && receipt.contract_address == Some(args.address) {
                &transaction.input
            } else if receipt.to == Some(DEFAULT_CREATE2_DEPLOYER) {
                &transaction.input[32..]
            } else {
                eyre::bail!(
                    "Could not extract the creation code for contract at address {}",
                    args.address
                );
            };

        // If bytecode_hash is disabled then its always partial verification
        let (verification_type, has_metadata) =
            match (&args.verification_type, config.bytecode_hash) {
                (VerificationType::Full, BytecodeHash::None) => (VerificationType::Partial, false),
                (VerificationType::Partial, BytecodeHash::None) => {
                    (VerificationType::Partial, false)
                }
                (VerificationType::Full, _) => (VerificationType::Full, true),
                (VerificationType::Partial, _) => (VerificationType::Partial, true),
            };

        trace!(target: "forge::verify", ?verification_type, has_metadata);

        // Etherscan compilation metadata
        let etherscan_metadata = source_code.items.first().unwrap();

        let local_bytecode = if let Some(local_bytecode) =
            prepare::build_using_cache(&args, etherscan_metadata, &config)
        {
            trace!("using cache");
            local_bytecode
        } else {
            prepare::build_project(&args, &config)?
        };

        // Append constructor args to the local_bytecode
        trace!(target: "forge::verify", %constructor_args);

        let mut local_bytecode_vec = local_bytecode.to_vec();
        local_bytecode_vec.extend_from_slice(&constructor_args);

        // Compare creation code with locally built bytecode and `maybe_creation_code`.
        let (did_match, with_status) = prepare::try_match(
            local_bytecode_vec.as_slice(),
            maybe_creation_code,
            &constructor_args,
            &verification_type,
            false,
            has_metadata,
        )?;

        let mut json_results: Vec<JsonResult> = vec![];
        prepare::print_result(
            &args,
            (did_match, with_status),
            BytecodeType::Creation,
            &mut json_results,
            etherscan_metadata,
            &config,
        );

        // If the creation code does not match, the runtime also won't match. Hence return.
        if !did_match {
            prepare::print_result(
                &args,
                (did_match, with_status),
                BytecodeType::Runtime,
                &mut json_results,
                etherscan_metadata,
                &config,
            );
            if args.json {
                println!("{}", serde_json::to_string(&json_results)?);
            }
            return Ok(());
        }

        // Get contract creation block.
        let simulation_block = match args.block {
            Some(BlockId::Number(BlockNumberOrTag::Number(block))) => block,
            Some(_) => eyre::bail!("Invalid block number"),
            None => {
                let provider = get_provider(&config)?;
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
        fork_config.fork_block_number = Some(simulation_block - 1);
        fork_config.evm_version =
            etherscan_metadata.evm_version()?.unwrap_or(EvmVersion::default());
        let (mut env, fork, _chain) =
            TracingExecutor::get_fork_material(&fork_config, evm_opts).await?;

        let mut executor =
            TracingExecutor::new(env.clone(), fork, Some(fork_config.evm_version), false, false);
        env.block.number = U256::from(simulation_block);
        let block = provider.get_block(simulation_block.into(), true.into()).await?;

        // Workaround for the `NonceTooHigh` issue as we're not simulating prior txs of the same
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

        // Replace the `input` with local creation code in the creation transaction.
        if let Some(to) = transaction.to {
            if to == DEFAULT_CREATE2_DEPLOYER {
                let mut input = transaction.input[..32].to_vec(); // SALT
                input.extend_from_slice(&local_bytecode_vec);
                transaction.input = Bytes::from(input);

                // Deploy default CREATE2 deployer.
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
            provider.get_code_at(args.address).block_id(BlockId::number(simulation_block)).await?;

        // Compare the onchain runtime bytecode with the runtime code from the fork.
        let (did_match, with_status) = prepare::try_match(
            &fork_runtime_code.original_bytes(),
            &onchain_runtime_code,
            &constructor_args,
            &verification_type,
            true,
            has_metadata,
        )?;

        prepare::print_result(
            &args,
            (did_match, with_status),
            BytecodeType::Runtime,
            &mut json_results,
            etherscan_metadata,
            &config,
        );

        if args.json {
            println!("{}", serde_json::to_string(&json_results)?);
        }

        Ok(())
    }

    /// Executes the command to check verification status on Etherscan
    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let config = args.try_load_config_emit_warnings()?;
        let etherscan = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key().as_deref(),
            &config,
        )?;
        let retry: Retry = args.retry.into();
        retry
            .run_async_until_break(|| {
                async {
                    let resp = etherscan
                        .check_contract_verification_status(args.id.clone())
                        .await
                        .wrap_err("Failed to request verification status")
                        .map_err(RetryError::Retry)?;

                    trace!(target: "forge::verify", ?resp, "Received verification response");

                    eprintln!(
                        "Contract verification status:\nResponse: `{}`\nDetails: `{}`",
                        resp.message, resp.result
                    );

                    if resp.result == "Pending in queue" {
                        return Err(RetryError::Retry(eyre!("Verification is still pending...",)))
                    }

                    if resp.result == "Unable to verify" {
                        return Err(RetryError::Retry(eyre!("Unable to verify.",)))
                    }

                    if resp.result == "Already Verified" {
                        println!("Contract source code already verified");
                        return Ok(())
                    }

                    if resp.status == "0" {
                        return Err(RetryError::Break(eyre!("Contract failed to verify.",)))
                    }

                    if resp.result == "Pass - Verified" {
                        println!("Contract successfully verified");
                    }

                    Ok(())
                }
                .boxed()
            })
            .await
            .wrap_err("Checking verification result failed")
    }
}

impl EtherscanVerificationProvider {
    /// Create a source provider
    fn source_provider(&self, args: &VerifyArgs) -> Box<dyn EtherscanSourceProvider> {
        if args.flatten {
            Box::new(flatten::EtherscanFlattenedSource)
        } else {
            Box::new(standard_json::EtherscanStandardJsonSource)
        }
    }

    /// Configures the API request to the Etherscan API using the given [`VerifyArgs`].
    async fn prepare_verify_request(
        &mut self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<(Client, VerifyContract)> {
        let config = args.try_load_config_emit_warnings()?;
        let etherscan = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key().as_deref(),
            &config,
        )?;
        let verify_args = self.create_verify_request(args, context).await?;

        Ok((etherscan, verify_args))
    }

    /// Configures the API request to the Etherscan API using the given [`VerifyBytecodeArgs`].
    async fn prepare_verify_bytecode_request(
        &mut self,
        args: &VerifyBytecodeArgs,
    ) -> Result<(Client, Config)> {
        let config = args.try_load_config_emit_warnings()?;
        let etherscan = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key().as_deref(),
            &config,
        )?;

        Ok((etherscan, config))
    }

    /// Queries the Etherscan API to verify if the contract is already verified.
    async fn is_contract_verified(
        &self,
        etherscan: &Client,
        verify_contract: &VerifyContract,
    ) -> Result<bool> {
        let check = etherscan.contract_abi(verify_contract.address).await;

        if let Err(err) = check {
            match err {
                EtherscanError::ContractCodeNotVerified(_) => return Ok(false),
                error => return Err(error.into()),
            }
        }

        Ok(true)
    }

    /// Create an Etherscan client.
    pub(crate) fn client(
        &self,
        chain: Chain,
        verifier_url: Option<&str>,
        etherscan_key: Option<&str>,
        config: &Config,
    ) -> Result<Client> {
        let etherscan_config = config.get_etherscan_config_with_chain(Some(chain))?;

        let etherscan_api_url = verifier_url
            .or_else(|| etherscan_config.as_ref().map(|c| c.api_url.as_str()))
            .map(str::to_owned);

        let api_url = etherscan_api_url.as_deref();
        let base_url = etherscan_config
            .as_ref()
            .and_then(|c| c.browser_url.as_deref())
            .or_else(|| chain.etherscan_urls().map(|(_, url)| url));

        let etherscan_key =
            etherscan_key.or_else(|| etherscan_config.as_ref().map(|c| c.key.as_str()));

        let mut builder = Client::builder();

        builder = if let Some(api_url) = api_url {
            // we don't want any trailing slashes because this can cause cloudflare issues: <https://github.com/foundry-rs/foundry/pull/6079>
            let api_url = api_url.trim_end_matches('/');
            builder
                .with_chain_id(chain)
                .with_api_url(api_url)?
                .with_url(base_url.unwrap_or(api_url))?
        } else {
            builder.chain(chain)?
        };

        builder
            .with_api_key(etherscan_key.unwrap_or_default())
            .build()
            .wrap_err("Failed to create Etherscan client")
    }

    /// Creates the `VerifyContract` Etherscan request in order to verify the contract
    ///
    /// If `--flatten` is set to `true` then this will send with [`CodeFormat::SingleFile`]
    /// otherwise this will use the [`CodeFormat::StandardJsonInput`]
    pub async fn create_verify_request(
        &mut self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<VerifyContract> {
        let (source, contract_name, code_format) =
            self.source_provider(args).source(args, context)?;

        let mut compiler_version = context.compiler_version.clone();
        compiler_version.build = match RE_BUILD_COMMIT.captures(compiler_version.build.as_str()) {
            Some(cap) => BuildMetadata::new(cap.name("commit").unwrap().as_str())?,
            _ => BuildMetadata::EMPTY,
        };

        let compiler_version =
            format!("v{}", ensure_solc_build_metadata(context.compiler_version.clone()).await?);
        let constructor_args = self.constructor_args(args, context).await?;
        let mut verify_args =
            VerifyContract::new(args.address, contract_name, source, compiler_version)
                .constructor_arguments(constructor_args)
                .code_format(code_format);

        if args.via_ir {
            // we explicitly set this __undocumented__ argument to true if provided by the user,
            // though this info is also available in the compiler settings of the standard json
            // object if standard json is used
            // unclear how Etherscan interprets this field in standard-json mode
            verify_args = verify_args.via_ir(true);
        }

        if code_format == CodeFormat::SingleFile {
            verify_args = if let Some(optimizations) = args.num_of_optimizations {
                verify_args.optimized().runs(optimizations as u32)
            } else if context.config.optimizer {
                verify_args.optimized().runs(context.config.optimizer_runs.try_into()?)
            } else {
                verify_args.not_optimized()
            };
        }

        Ok(verify_args)
    }

    /// Return the optional encoded constructor arguments. If the path to
    /// constructor arguments was provided, read them and encode. Otherwise,
    /// return whatever was set in the [VerifyArgs] args.
    async fn constructor_args(
        &mut self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<Option<String>> {
        if let Some(ref constructor_args_path) = args.constructor_args_path {
            let abi = context.get_target_abi()?;
            let constructor = abi
                .constructor()
                .ok_or_else(|| eyre!("Can't retrieve constructor info from artifact ABI."))?;
            #[allow(deprecated)]
            let func = Function {
                name: "constructor".to_string(),
                inputs: constructor.inputs.clone(),
                outputs: vec![],
                state_mutability: alloy_json_abi::StateMutability::NonPayable,
            };
            let encoded_args = encode_function_args(
                &func,
                read_constructor_args_file(constructor_args_path.to_path_buf())?,
            )?;
            let encoded_args = hex::encode(encoded_args);
            return Ok(Some(encoded_args[8..].into()))
        }
        if args.guess_constructor_args {
            return Ok(Some(self.guess_constructor_args(args, context).await?))
        }

        Ok(args.constructor_args.clone())
    }

    /// Uses Etherscan API to fetch contract creation transaction.
    /// If transaction is a create transaction or a invocation of default CREATE2 deployer, tries to
    /// match provided creation code with local bytecode of the target contract.
    /// If bytecode match, returns latest bytes of on-chain creation code as constructor arguments.
    async fn guess_constructor_args(
        &mut self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<String> {
        let provider = get_provider(&context.config)?;
        let client = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key.as_deref(),
            &context.config,
        )?;

        let creation_data = client.contract_creation_data(args.address).await?;
        let transaction = provider
            .get_transaction_by_hash(creation_data.transaction_hash)
            .await?
            .ok_or_eyre("Transaction not found")?;
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash)
            .await?
            .ok_or_eyre("Couldn't fetch transaction receipt from RPC")?;

        let maybe_creation_code = if receipt.contract_address == Some(args.address) {
            &transaction.input
        } else if transaction.to == Some(DEFAULT_CREATE2_DEPLOYER) {
            &transaction.input[32..]
        } else {
            eyre::bail!("Fetching of constructor arguments is not supported for contracts created by contracts")
        };

        let output = context.project.compile_file(&context.target_path)?;
        let artifact = output
            .find(&context.target_path, &context.target_name)
            .ok_or_eyre("Contract artifact wasn't found locally")?;
        let bytecode = artifact
            .get_bytecode_object()
            .ok_or_eyre("Contract artifact does not contain bytecode")?;

        let bytecode = match bytecode.as_ref() {
            BytecodeObject::Bytecode(bytes) => Ok(bytes),
            BytecodeObject::Unlinked(_) => {
                Err(eyre!("You have to provide correct libraries to use --guess-constructor-args"))
            }
        }?;

        if maybe_creation_code.starts_with(bytecode) {
            let constructor_args = &maybe_creation_code[bytecode.len()..];
            let constructor_args = hex::encode(constructor_args);
            shell::println(format!("Identified constructor arguments: {constructor_args}"))?;
            Ok(constructor_args)
        } else {
            eyre::bail!("Local bytecode doesn't match on-chain bytecode")
        }
    }
}

/// Given any solc [Version] return a [Version] with build metadata
///
/// # Example
///
/// ```ignore
/// use semver::{BuildMetadata, Version};
/// let version = Version::new(1, 2, 3);
/// let version = ensure_solc_build_metadata(version).await?;
/// assert_ne!(version.build, BuildMetadata::EMPTY);
/// ```
async fn ensure_solc_build_metadata(version: Version) -> Result<Version> {
    if version.build != BuildMetadata::EMPTY {
        Ok(version)
    } else {
        Ok(lookup_compiler_version(&version).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use foundry_common::fs;
    use foundry_test_utils::forgetest_async;
    use tempfile::tempdir;

    #[test]
    fn can_extract_etherscan_verify_config() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]

                [etherscan]
                mumbai = { key = "dummykey", chain = 80001, url = "https://api-testnet.polygonscan.com/" }
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();

        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0xd8509bee9c9bf012282ad33aba0d87241baf5064",
            "src/Counter.sol:Counter",
            "--chain",
            "mumbai",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let config = args.load_config();

        let etherscan = EtherscanVerificationProvider::default();
        let client = etherscan
            .client(
                args.etherscan.chain.unwrap_or_default(),
                args.verifier.verifier_url.as_deref(),
                args.etherscan.key().as_deref(),
                &config,
            )
            .unwrap();
        assert_eq!(client.etherscan_api_url().as_str(), "https://api-testnet.polygonscan.com/");

        assert!(format!("{client:?}").contains("dummykey"));

        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0xd8509bee9c9bf012282ad33aba0d87241baf5064",
            "src/Counter.sol:Counter",
            "--chain",
            "mumbai",
            "--verifier-url",
            "https://verifier-url.com/",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let config = args.load_config();

        let etherscan = EtherscanVerificationProvider::default();
        let client = etherscan
            .client(
                args.etherscan.chain.unwrap_or_default(),
                args.verifier.verifier_url.as_deref(),
                args.etherscan.key().as_deref(),
                &config,
            )
            .unwrap();
        assert_eq!(client.etherscan_api_url().as_str(), "https://verifier-url.com/");
        assert!(format!("{client:?}").contains("dummykey"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_on_disabled_cache_and_missing_info() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let root_path = root.as_os_str().to_str().unwrap();

        let config = r"
                [profile.default]
                cache = false
            ";

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();

        let address = "0xd8509bee9c9bf012282ad33aba0d87241baf5064";
        let contract_name = "Counter";
        let src_dir = "src";
        fs::create_dir_all(root.join(src_dir)).unwrap();
        let contract_path = format!("{src_dir}/Counter.sol");
        fs::write(root.join(&contract_path), "").unwrap();

        // No compiler argument
        let args = VerifyArgs::parse_from([
            "foundry-cli",
            address,
            &format!("{contract_path}:{contract_name}"),
            "--root",
            root_path,
        ]);
        let result = args.resolve_context().await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
        );
    }

    forgetest_async!(respects_path_for_duplicate, |prj, cmd| {
        prj.add_source("Counter1", "contract Counter {}").unwrap();
        prj.add_source("Counter2", "contract Counter {}").unwrap();

        cmd.args(["build", "--force"]).ensure_execute_success().unwrap();

        let args = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter1.sol:Counter",
            "--root",
            &prj.root().to_string_lossy(),
        ]);
        let context = args.resolve_context().await.unwrap();

        let mut etherscan = EtherscanVerificationProvider::default();
        etherscan.preflight_verify_check(args, context).await.unwrap();
    });
}
