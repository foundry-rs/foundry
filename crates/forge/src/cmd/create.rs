use crate::cmd::install;
use alloy_chains::Chain;
use alloy_consensus::{SignableTransaction, Signed};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt, Specifier};
use alloy_json_abi::{Constructor, JsonAbi};
use alloy_network::{Ethereum, EthereumWallet, Network, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, Bytes, U256, hex};
use alloy_provider::{PendingTransactionError, Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::{Signature, Signer};
use alloy_transport::TransportError;
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use forge_verify::{RetryArgs, VerifierArgs, VerifyArgs};
use foundry_cli::{
    opts::{BuildOpts, EthereumOpts, EtherscanOpts, TransactionOpts},
    utils::{LoadConfig, find_contract_artifacts, read_constructor_args_file},
};
use foundry_common::{
    FoundryTransactionBuilder,
    compile::{self},
    fmt::parse_tokens,
    provider::ProviderBuilder,
    shell,
};
use foundry_compilers::{
    ArtifactId, artifacts::BytecodeObject, info::ContractInfo, utils::canonicalize,
};
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
    merge_impl_figment_convert,
};
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use serde_json::json;
use std::{borrow::Borrow, marker::PhantomData, path::PathBuf, sync::Arc, time::Duration};
use tempo_alloy::TempoNetwork;

merge_impl_figment_convert!(CreateArgs, build, eth);

/// CLI arguments for `forge create`.
#[derive(Clone, Debug, Parser)]
pub struct CreateArgs {
    /// The contract identifier in the form `<path>:<contractname>`.
    contract: ContractInfo,

    /// The constructor arguments.
    #[arg(
        long,
        num_args(1..),
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
        allow_hyphen_values = true,
    )]
    constructor_args: Vec<String>,

    /// The path to a file containing the constructor arguments.
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
    )]
    constructor_args_path: Option<PathBuf>,

    /// Broadcast the transaction.
    #[arg(long)]
    pub broadcast: bool,

    /// Verify contract after creation.
    #[arg(long)]
    verify: bool,

    /// Send via `eth_sendTransaction` using the `--from` argument or `$ETH_FROM` as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    /// Prints the standard json compiler input if `--verify` is provided.
    ///
    /// The standard json compiler input can be used to manually submit contract verification in
    /// the browser.
    #[arg(long, requires = "verify")]
    show_standard_json_input: bool,

    /// Timeout to use for broadcasting transactions.
    #[arg(long, env = "ETH_TIMEOUT")]
    pub timeout: Option<u64>,

    #[command(flatten)]
    build: BuildOpts,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,

    #[command(flatten)]
    pub verifier: VerifierArgs,

    #[command(flatten)]
    retry: RetryArgs,
}

impl CreateArgs {
    /// Executes the command to create a contract
    pub async fn run(self) -> Result<()> {
        let (signer, tempo_access_key) = self.eth.wallet.maybe_signer().await?;

        if tempo_access_key.is_some() || self.tx.tempo.is_tempo() {
            self.run_generic::<TempoNetwork>(signer, tempo_access_key).await
        } else {
            self.run_generic::<Ethereum>(signer, None).await
        }
    }

    async fn run_generic<N: Network>(
        mut self,
        pre_resolved_signer: Option<WalletSigner>,
        access_key: Option<TempoAccessKeyConfig>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
        N::ReceiptResponse: serde::Serialize,
    {
        let mut config = self.load_config()?;

        // Install missing dependencies.
        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

        // Find Project & Compile
        let project = config.project()?;

        let target_path = if let Some(ref mut path) = self.contract.path {
            canonicalize(project.root().join(path))?
        } else {
            project.find_contract_path(&self.contract.name)?
        };

        let output = compile::compile_target(&target_path, &project, shell::is_json())?;

        let (abi, bin, id) = find_contract_artifacts(output, &target_path, &self.contract.name)?;

        let bin = match bin.object {
            BytecodeObject::Bytecode(_) => bin.object,
            _ => {
                let link_refs = bin
                    .link_references
                    .iter()
                    .flat_map(|(path, names)| {
                        names.keys().map(move |name| format!("\t{name}: {path}"))
                    })
                    .collect::<Vec<String>>()
                    .join("\n");
                eyre::bail!(
                    "Dynamic linking not supported in `create` command - deploy the following library contracts first, then provide the address to link at compile time\n{}",
                    link_refs
                )
            }
        };

        // Add arguments to constructor
        let params = if let Some(constructor) = &abi.constructor {
            let constructor_args =
                self.constructor_args_path.clone().map(read_constructor_args_file).transpose()?;
            self.parse_constructor_args(
                constructor,
                constructor_args.as_deref().unwrap_or(&self.constructor_args),
            )?
        } else {
            vec![]
        };

        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;

        // Inject access key ID into TempoOpts so it's set before gas estimation.
        if let Some(ref ak) = access_key {
            self.tx.tempo.key_id = Some(ak.key_address);
        }

        // respect chain, if set explicitly via cmd args
        let chain_id = if let Some(chain_id) = self.chain_id() {
            chain_id
        } else {
            provider.get_chain_id().await?
        };

        // Whether to broadcast the transaction or not
        let dry_run = !self.broadcast;

        if self.unlocked {
            // Deploy with unlocked account
            let sender = self.eth.wallet.from.expect("required");
            self.deploy(
                abi,
                bin,
                params,
                provider,
                chain_id,
                sender,
                config.transaction_timeout,
                id,
                dry_run,
                None,
            )
            .await
        } else if let Some(ak) = access_key {
            // Tempo keychain mode: sign with access key and send raw
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => self.eth.wallet.signer().await?,
            };
            let deployer_address = ak.wallet_address;
            self.deploy(
                abi,
                bin,
                params,
                provider,
                chain_id,
                deployer_address,
                config.transaction_timeout,
                id,
                dry_run,
                Some((signer, ak)),
            )
            .await
        } else {
            // Deploy with signer
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => self.eth.wallet.signer().await?,
            };
            let deployer = signer.address();
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(EthereumWallet::new(signer))
                .connect_provider(provider);
            self.deploy(
                abi,
                bin,
                params,
                provider,
                chain_id,
                deployer,
                config.transaction_timeout,
                id,
                dry_run,
                None,
            )
            .await
        }
    }

    /// Returns the provided chain id, if any.
    fn chain_id(&self) -> Option<u64> {
        self.eth.etherscan.chain.map(|chain| chain.id())
    }

    /// Ensures the verify command can be executed.
    ///
    /// This is supposed to check any things that might go wrong when preparing a verify request
    /// before the contract is deployed. This should prevent situations where a contract is deployed
    /// successfully, but we fail to prepare a verify request which would require manual
    /// verification.
    async fn verify_preflight_check(
        &self,
        constructor_args: Option<String>,
        chain: u64,
        id: &ArtifactId,
    ) -> Result<()> {
        // NOTE: this does not represent the same `VerifyArgs` that would be sent after deployment,
        // since we don't know the address yet.
        let mut verify = VerifyArgs {
            address: Default::default(),
            contract: Some(self.contract.clone()),
            compiler_version: Some(id.version.to_string()),
            constructor_args,
            constructor_args_path: None,
            no_auto_detect: false,
            use_solc: None,
            num_of_optimizations: None,
            etherscan: EtherscanOpts {
                key: self.eth.etherscan.key.clone(),
                chain: Some(chain.into()),
            },
            rpc: Default::default(),
            flatten: false,
            force: false,
            skip_is_verified_check: true,
            watch: true,
            retry: self.retry,
            libraries: self.build.libraries.clone(),
            root: None,
            verifier: self.verifier.clone(),
            via_ir: self.build.via_ir,
            evm_version: self.build.compiler.evm_version,
            show_standard_json_input: self.show_standard_json_input,
            guess_constructor_args: false,
            compilation_profile: Some(id.profile.clone()),
            language: None,
            creation_transaction_hash: None,
        };

        // Check config for Etherscan API Keys to avoid preflight check failing if no
        // ETHERSCAN_API_KEY value set.
        let config = verify.load_config()?;
        verify.etherscan.key =
            config.get_etherscan_config_with_chain(Some(chain.into()))?.map(|c| c.key);

        let context = verify.resolve_context().await?;

        verify.verification_provider()?.preflight_verify_check(verify, context).await?;
        Ok(())
    }

    /// Deploys the contract
    #[expect(clippy::too_many_arguments)]
    async fn deploy<N: Network, P: Provider<N>>(
        self,
        abi: JsonAbi,
        bin: BytecodeObject,
        args: Vec<DynSolValue>,
        provider: P,
        chain: u64,
        deployer_address: Address,
        timeout: u64,
        id: ArtifactId,
        dry_run: bool,
        tempo_keychain: Option<(WalletSigner, TempoAccessKeyConfig)>,
    ) -> Result<()>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
        N::ReceiptResponse: serde::Serialize,
    {
        let bin = bin.into_bytes().unwrap_or_default();
        if bin.is_empty() {
            eyre::bail!("no bytecode found in bin object for {}", self.contract.name)
        }

        let provider = Arc::new(provider);
        let factory =
            ContractFactory::<N, _>::new(abi.clone(), bin.clone(), provider.clone(), timeout);

        let is_args_empty = args.is_empty();
        let mut deployer =
            factory.deploy_tokens(args.clone(), self.tx.tempo.fee_token).context("failed to deploy contract").map_err(|e| {
                if is_args_empty {
                    e.wrap_err("no arguments provided for contract constructor; consider --constructor-args or --constructor-args-path")
                } else {
                    e
                }
            })?;
        let is_legacy = self.tx.legacy || Chain::from(chain).is_legacy();

        deployer.tx.set_from(deployer_address);
        deployer.tx.set_chain_id(chain);
        // `to` field must be set explicitly, cannot be None.
        if deployer.tx.to().is_none() {
            deployer.tx.set_create();
        }

        // Apply user-provided gas, fee, nonce, and Tempo options.
        self.tx.apply::<N>(&mut deployer.tx, is_legacy);

        // For keychain mode, set key_id and nonce_key before gas estimation.
        // Convert the CREATE into an AA-compatible call entry since Tempo AA
        // transactions use a `calls` list instead of `to`+`input`.
        if let Some((_, ref ak)) = tempo_keychain {
            deployer.tx.set_key_id(ak.key_address);
            if deployer.tx.nonce_key().is_none() {
                deployer.tx.set_nonce_key(U256::ZERO);
            }
            deployer.tx.convert_create_to_call();
        }

        // Fetch defaults from provider for values not specified by user.
        if self.tx.nonce.is_none() && !self.tx.tempo.expiring_nonce {
            deployer.tx.set_nonce(provider.get_transaction_count(deployer_address).await?);
        }

        // set access list if specified
        if let Some(access_list) = match self.tx.access_list {
            None => None,
            Some(None) => Some(provider.create_access_list(&deployer.tx).await?.access_list),
            Some(Some(ref access_list)) => Some(access_list.clone()),
        } {
            deployer.tx.set_access_list(access_list);
        }

        if self.tx.gas_limit.is_none() {
            deployer.tx.set_gas_limit(provider.estimate_gas(deployer.tx.clone()).await?);
        }

        if is_legacy {
            if self.tx.gas_price.is_none() {
                deployer.tx.set_gas_price(provider.get_gas_price().await?);
            }
        } else if self.tx.gas_price.is_none() || self.tx.priority_gas_price.is_none() {
            let estimate = provider.estimate_eip1559_fees().await.wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;
            if self.tx.priority_gas_price.is_none() {
                deployer.tx.set_max_priority_fee_per_gas(estimate.max_priority_fee_per_gas);
            }
            if self.tx.gas_price.is_none() {
                deployer.tx.set_max_fee_per_gas(estimate.max_fee_per_gas);
            }
        }

        // Before we actually deploy the contract we try check if the verify settings are valid
        let mut constructor_args = None;
        if self.verify {
            if !args.is_empty() {
                let encoded_args = abi
                    .constructor()
                    .ok_or_else(|| eyre::eyre!("could not find constructor"))?
                    .abi_encode_input(&args)?;
                constructor_args = Some(hex::encode(encoded_args));
            }

            self.verify_preflight_check(constructor_args.clone(), chain, &id).await?;
        }

        if dry_run {
            if shell::is_json() {
                let output = json!({
                    "contract": self.contract.name,
                    "transaction": &deployer.tx,
                    "abi":&abi
                });
                sh_println!("{}", serde_json::to_string_pretty(&output)?)?;
            } else {
                sh_warn!("Dry run enabled, not broadcasting transaction\n")?;

                sh_println!("Contract: {}", self.contract.name)?;
                sh_println!(
                    "Transaction: {}",
                    serde_json::to_string_pretty(&deployer.tx.clone())?
                )?;
                sh_println!("ABI: {}\n", serde_json::to_string_pretty(&abi)?)?;

                sh_warn!(
                    "To broadcast this transaction, add --broadcast to the previous command. See forge create --help for more."
                )?;
            }

            return Ok(());
        }

        // Deploy the actual contract
        let (deployed_contract, receipt) = if let Some((signer, ak)) = tempo_keychain {
            // Tempo keychain mode: sign with access key provisioning and send raw
            let raw_tx = deployer
                .tx
                .sign_with_access_key(
                    &provider,
                    &signer,
                    ak.wallet_address,
                    ak.key_address,
                    ak.key_authorization.as_ref(),
                )
                .await?;

            let receipt = provider
                .send_raw_transaction(&raw_tx)
                .await?
                .with_required_confirmations(1)
                .with_timeout(Some(Duration::from_secs(timeout)))
                .get_receipt()
                .await?;

            let address = receipt
                .contract_address()
                .ok_or_else(|| eyre::eyre!("contract was not deployed"))?;

            (address, receipt)
        } else {
            deployer.send_with_receipt().await?
        };

        let address = deployed_contract;
        let tx_hash = receipt.transaction_hash();
        if shell::is_json() {
            let output = json!({
                "deployer": deployer_address.to_string(),
                "deployedTo": address.to_string(),
                "transactionHash": tx_hash
            });
            sh_println!("{}", serde_json::to_string_pretty(&output)?)?;
        } else {
            sh_println!("Deployer: {deployer_address}")?;
            sh_println!("Deployed to: {address}")?;
            sh_println!("Transaction hash: {tx_hash:?}")?;
        };

        if !self.verify {
            return Ok(());
        }

        sh_println!("Starting contract verification...")?;

        let num_of_optimizations = if let Some(optimizer) = self.build.compiler.optimize {
            optimizer.then(|| self.build.compiler.optimizer_runs.unwrap_or(200))
        } else {
            self.build.compiler.optimizer_runs
        };

        let verify = VerifyArgs {
            address,
            contract: Some(self.contract),
            compiler_version: Some(id.version.to_string()),
            constructor_args,
            constructor_args_path: None,
            no_auto_detect: false,
            use_solc: None,
            num_of_optimizations,
            etherscan: EtherscanOpts { key: self.eth.etherscan.key(), chain: Some(chain.into()) },
            rpc: Default::default(),
            flatten: false,
            force: false,
            skip_is_verified_check: true,
            watch: true,
            retry: self.retry,
            libraries: self.build.libraries.clone(),
            root: None,
            verifier: self.verifier,
            via_ir: self.build.via_ir,
            evm_version: self.build.compiler.evm_version,
            show_standard_json_input: self.show_standard_json_input,
            guess_constructor_args: false,
            compilation_profile: Some(id.profile.clone()),
            language: None,
            creation_transaction_hash: Some(tx_hash),
        };
        sh_println!("Waiting for {} to detect contract deployment...", verify.verifier.verifier)?;
        verify.run().await
    }

    /// Parses the given constructor arguments into a vector of `DynSolValue`s, by matching them
    /// against the constructor's input params.
    ///
    /// Returns a list of parsed values that match the constructor's input params.
    fn parse_constructor_args(
        &self,
        constructor: &Constructor,
        constructor_args: &[String],
    ) -> Result<Vec<DynSolValue>> {
        if constructor.inputs.len() != constructor_args.len() {
            eyre::bail!(
                "Constructor argument count mismatch: expected {} but got {}",
                constructor.inputs.len(),
                constructor_args.len()
            );
        }

        let mut params = Vec::with_capacity(constructor.inputs.len());
        for (input, arg) in constructor.inputs.iter().zip(constructor_args) {
            // resolve the input type directly
            let ty = input
                .resolve()
                .wrap_err_with(|| format!("Could not resolve constructor arg: input={input}"))?;
            params.push((ty, arg));
        }
        let params = params.iter().map(|(ty, arg)| (ty, arg.as_str()));
        parse_tokens(params).map_err(Into::into)
    }
}

impl figment::Provider for CreateArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Create Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();
        if let Some(timeout) = self.timeout {
            dict.insert("transaction_timeout".to_string(), timeout.into());
        }
        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// `ContractFactory` is a [`DeploymentTxFactory`] object with an
/// [`Arc`] middleware. This type alias exists to preserve backwards
/// compatibility with less-abstract Contracts.
///
/// For full usage docs, see [`DeploymentTxFactory`].
pub type ContractFactory<N, P> = DeploymentTxFactory<N, P>;

/// Helper which manages the deployment transaction of a smart contract. It
/// wraps a deployment transaction, and retrieves the contract address output
/// by it.
#[derive(Debug)]
#[must_use = "ContractDeploymentTx does nothing unless you `send` it"]
pub struct ContractDeploymentTx<N: Network, P, C> {
    /// the actual deployer, exposed for overriding the defaults
    pub deployer: Deployer<N, P>,
    /// marker for the `Contract` type to create afterwards
    ///
    /// this type will be used to construct it via `From::from(Contract)`
    _contract: PhantomData<C>,
}

impl<N: Network, P: Clone, C> Clone for ContractDeploymentTx<N, P, C> {
    fn clone(&self) -> Self {
        Self { deployer: self.deployer.clone(), _contract: self._contract }
    }
}

impl<N: Network, P, C> From<Deployer<N, P>> for ContractDeploymentTx<N, P, C> {
    fn from(deployer: Deployer<N, P>) -> Self {
        Self { deployer, _contract: PhantomData }
    }
}

/// Helper which manages the deployment transaction of a smart contract
#[derive(Clone, Debug)]
#[must_use = "Deployer does nothing unless you `send` it"]
pub struct Deployer<N: Network, P> {
    /// The deployer's transaction, exposed for overriding the defaults
    pub tx: N::TransactionRequest,
    client: P,
    confs: usize,
    timeout: u64,
}

impl<N: Network, P: Provider<N>> Deployer<N, P> {
    /// Broadcasts the contract deployment transaction and after waiting for it to
    /// be sufficiently confirmed (default: 1), it returns a tuple with the [`Address`] at the
    /// deployed contract's address and the corresponding receipt.
    pub async fn send_with_receipt(
        self,
    ) -> Result<(Address, N::ReceiptResponse), ContractDeploymentError> {
        let receipt = self
            .client
            .borrow()
            .send_transaction(self.tx)
            .await?
            .with_required_confirmations(self.confs as u64)
            .with_timeout(Some(Duration::from_secs(self.timeout)))
            .get_receipt()
            .await?;

        if !receipt.status() {
            return Err(ContractDeploymentError::DeploymentFailed(receipt.transaction_hash()));
        }

        let address =
            receipt.contract_address().ok_or(ContractDeploymentError::ContractNotDeployed)?;

        Ok((address, receipt))
    }
}

/// To deploy a contract to the Ethereum network, a [`ContractFactory`] can be
/// created which manages the Contract bytecode and Application Binary Interface
/// (ABI), usually generated from the Solidity compiler.
#[derive(Clone, Debug)]
pub struct DeploymentTxFactory<N: Network, P> {
    client: P,
    abi: JsonAbi,
    bytecode: Bytes,
    timeout: u64,
    _network: PhantomData<N>,
}

impl<N: Network, P: Provider<N> + Clone> DeploymentTxFactory<N, P> {
    /// Creates a factory for deployment of the Contract with bytecode, and the
    /// constructor defined in the abi. The client will be used to send any deployment
    /// transaction.
    pub fn new(abi: JsonAbi, bytecode: Bytes, client: P, timeout: u64) -> Self {
        Self { client, abi, bytecode, timeout, _network: PhantomData }
    }

    /// Create a deployment tx using the provided tokens as constructor
    /// arguments
    pub fn deploy_tokens(
        self,
        params: Vec<DynSolValue>,
        fee_token: Option<Address>,
    ) -> Result<Deployer<N, P>, ContractDeploymentError>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        // Encode the constructor args & concatenate with the bytecode if necessary
        let data: Bytes = match (self.abi.constructor(), params.is_empty()) {
            (None, false) => return Err(ContractDeploymentError::ConstructorError),
            (None, true) => self.bytecode.clone(),
            (Some(constructor), _) => {
                let input: Bytes = constructor
                    .abi_encode_input(&params)
                    .map_err(ContractDeploymentError::DetokenizationError)?
                    .into();
                // Concatenate the bytecode and abi-encoded constructor call.
                self.bytecode.iter().copied().chain(input).collect()
            }
        };

        // create the tx object. Since we're deploying a contract, `to` is `None`
        let mut tx = N::TransactionRequest::default();
        tx.set_input(data);
        if let Some(fee_token) = fee_token {
            tx.set_fee_token(fee_token);
        }
        Ok(Deployer { client: self.client.clone(), tx, confs: 1, timeout: self.timeout })
    }
}

#[derive(thiserror::Error, Debug)]
/// An Error which is thrown when interacting with a smart contract
pub enum ContractDeploymentError {
    #[error("constructor is not defined in the ABI")]
    ConstructorError,
    #[error(transparent)]
    DetokenizationError(#[from] alloy_dyn_abi::Error),
    #[error("contract was not deployed")]
    ContractNotDeployed,
    #[error("deployment transaction failed (receipt status 0): {0}")]
    DeploymentFailed(alloy_primitives::TxHash),
    #[error(transparent)]
    RpcError(#[from] TransportError),
}

impl From<PendingTransactionError> for ContractDeploymentError {
    fn from(_err: PendingTransactionError) -> Self {
        Self::ContractNotDeployed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::I256;

    #[test]
    fn can_parse_create() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--verify",
            "--retries",
            "10",
            "--delay",
            "30",
        ]);
        assert_eq!(args.retry.retries, 10);
        assert_eq!(args.retry.delay, 30);
    }
    #[test]
    fn can_parse_chain_id() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--verify",
            "--retries",
            "10",
            "--delay",
            "30",
            "--chain-id",
            "9999",
        ]);
        assert_eq!(args.chain_id(), Some(9999));
    }

    #[test]
    fn test_parse_constructor_args() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--constructor-args",
            "Hello",
        ]);
        let constructor: Constructor = serde_json::from_str(r#"{"type":"constructor","inputs":[{"name":"_name","type":"string","internalType":"string"}],"stateMutability":"nonpayable"}"#).unwrap();
        let params = args.parse_constructor_args(&constructor, &args.constructor_args).unwrap();
        assert_eq!(params, vec![DynSolValue::String("Hello".to_string())]);
    }

    #[test]
    fn test_parse_tuple_constructor_args() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--constructor-args",
            "[(1,2), (2,3), (3,4)]",
        ]);
        let constructor: Constructor = serde_json::from_str(r#"{"type":"constructor","inputs":[{"name":"_points","type":"tuple[]","internalType":"struct Point[]","components":[{"name":"x","type":"uint256","internalType":"uint256"},{"name":"y","type":"uint256","internalType":"uint256"}]}],"stateMutability":"nonpayable"}"#).unwrap();
        let _params = args.parse_constructor_args(&constructor, &args.constructor_args).unwrap();
    }

    #[test]
    fn test_parse_int_constructor_args() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--constructor-args",
            "-5",
        ]);
        let constructor: Constructor = serde_json::from_str(r#"{"type":"constructor","inputs":[{"name":"_name","type":"int256","internalType":"int256"}],"stateMutability":"nonpayable"}"#).unwrap();
        let params = args.parse_constructor_args(&constructor, &args.constructor_args).unwrap();
        assert_eq!(params, vec![DynSolValue::Int(I256::unchecked_from(-5), 256)]);
    }
}
