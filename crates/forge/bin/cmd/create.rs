use super::{retry::RetryArgs, verify};
use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::{Constructor, JsonAbi as Abi};
use alloy_primitives::{Address, Bytes, U256, U64};
use clap::{Parser, ValueHint};
use ethers::{
    abi::InvalidOutputType,
    prelude::{Middleware, MiddlewareBuilder},
    types::{transaction::eip2718::TypedTransaction, Chain},
};
use eyre::{Context, Result};
use foundry_cli::{
    opts::{CoreBuildArgs, EthereumOpts, EtherscanOpts, TransactionOpts},
    utils::{self, read_constructor_args_file, remove_contract, LoadConfig},
};
use foundry_common::{abi::parse_tokens, compile, estimate_eip1559_fees};
use foundry_compilers::{artifacts::BytecodeObject, info::ContractInfo, utils::canonicalized};
use foundry_utils::types::{ToAlloy, ToEthers};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

/// CLI arguments for `forge create`.
#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    /// The contract identifier in the form `<path>:<contractname>`.
    contract: ContractInfo,

    /// The constructor arguments.
    #[clap(
        long,
        num_args(1..),
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
    )]
    constructor_args: Vec<String>,

    /// The path to a file containing the constructor arguments.
    #[clap(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
    )]
    constructor_args_path: Option<PathBuf>,

    /// Print the deployment information as JSON.
    #[clap(long, help_heading = "Display options")]
    json: bool,

    /// Verify contract after creation.
    #[clap(long)]
    verify: bool,

    /// Send via `eth_sendTransaction` using the `--from` argument or `$ETH_FROM` as sender
    #[clap(long, requires = "from")]
    unlocked: bool,

    #[clap(flatten)]
    opts: CoreBuildArgs,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
    eth: EthereumOpts,

    #[clap(flatten)]
    pub verifier: verify::VerifierArgs,

    #[clap(flatten)]
    retry: RetryArgs,
}

impl CreateArgs {
    /// Executes the command to create a contract
    pub async fn run(mut self) -> Result<()> {
        // Find Project & Compile
        let project = self.opts.project()?;
        let mut output = if self.json || self.opts.silent {
            // Suppress compile stdout messages when printing json output or when silent
            compile::suppress_compile(&project)
        } else {
            compile::compile(&project, false, false)
        }?;

        if let Some(ref mut path) = self.contract.path {
            // paths are absolute in the project's output
            *path = canonicalized(project.root().join(&path)).to_string_lossy().to_string();
        }

        let (abi, bin, _) = remove_contract(&mut output, &self.contract)?;

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
                eyre::bail!("Dynamic linking not supported in `create` command - deploy the following library contracts first, then provide the address to link at compile time\n{}", link_refs)
            }
        };

        // Add arguments to constructor
        let config = self.eth.try_load_config_emit_warnings()?;
        let provider = utils::get_provider(&config)?;
        let params = match abi.constructor {
            Some(ref v) => {
                let constructor_args =
                    if let Some(ref constructor_args_path) = self.constructor_args_path {
                        read_constructor_args_file(constructor_args_path.to_path_buf())?
                    } else {
                        self.constructor_args.clone()
                    };
                self.parse_constructor_args(v, &constructor_args)?
            }
            None => vec![],
        };

        // respect chain, if set explicitly via cmd args
        let chain_id = if let Some(chain_id) = self.chain_id() {
            chain_id
        } else {
            provider.get_chainid().await?.as_u64()
        };
        if self.unlocked {
            // Deploy with unlocked account
            let sender = self.eth.wallet.from.expect("required");
            let provider = provider.with_sender(sender);
            self.deploy(abi, bin, params, provider, chain_id).await
        } else {
            // Deploy with signer
            let signer = self.eth.wallet.signer(chain_id).await?;
            let provider = provider.with_signer(signer);
            self.deploy(abi, bin, params, provider, chain_id).await
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
    ) -> Result<()> {
        // NOTE: this does not represent the same `VerifyArgs` that would be sent after deployment,
        // since we don't know the address yet.
        let verify = verify::VerifyArgs {
            address: Default::default(),
            contract: self.contract.clone(),
            compiler_version: None,
            constructor_args,
            constructor_args_path: None,
            num_of_optimizations: None,
            etherscan: EtherscanOpts {
                key: self.eth.etherscan.key.clone(),
                chain: Some(chain.into()),
            },
            flatten: false,
            force: false,
            watch: true,
            retry: self.retry,
            libraries: vec![],
            root: None,
            verifier: self.verifier.clone(),
            show_standard_json_input: false,
        };
        verify.verification_provider()?.preflight_check(verify).await?;
        Ok(())
    }

    /// Deploys the contract
    async fn deploy<M: Middleware + 'static>(
        self,
        abi: Abi,
        bin: BytecodeObject,
        args: Vec<DynSolValue>,
        provider: M,
        chain: u64,
    ) -> Result<()> {
        let deployer_address =
            provider.default_sender().expect("no sender address set for provider");
        let bin = bin.into_bytes().unwrap_or_else(|| {
            panic!("no bytecode found in bin object for {}", self.contract.name)
        });
        let provider = Arc::new(provider);
        let factory = ContractFactory::new(abi.clone(), bin.clone().0.into(), provider.clone());

        let is_args_empty = args.is_empty();
        let deployer =
            factory.deploy_tokens(args.clone()).context("Failed to deploy contract").map_err(|e| {
                if is_args_empty {
                    e.wrap_err("No arguments provided for contract constructor. Consider --constructor-args or --constructor-args-path")
                } else {
                    e
                }
            })?;
        let is_legacy = self.tx.legacy ||
            Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default();
        let mut deployer = if is_legacy { deployer.legacy() } else { deployer };

        // set tx value if specified
        if let Some(value) = self.tx.value {
            deployer.tx.set_value(value);
        }

        // fill tx first because if you target a lower gas than current base, eth_estimateGas
        // will fail and create will fail
        provider.fill_transaction(&mut deployer.tx, None).await?;

        // the max
        let mut priority_fee = self.tx.priority_gas_price;

        // set gas price if specified
        if let Some(gas_price) = self.tx.gas_price {
            deployer.tx.set_gas_price(gas_price);
        } else if !is_legacy {
            // estimate EIP1559 fees
            let (max_fee, max_priority_fee) = estimate_eip1559_fees(&provider, Some(chain))
                .await
                .wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;
            deployer.tx.set_gas_price(max_fee);
            if priority_fee.is_none() {
                priority_fee = Some(max_priority_fee);
            }
        }

        // set gas limit if specified
        if let Some(gas_limit) = self.tx.gas_limit {
            deployer.tx.set_gas(gas_limit);
        }

        // set nonce if specified
        if let Some(nonce) = self.tx.nonce {
            deployer.tx.set_nonce(nonce);
        }

        // set priority fee if specified
        if let Some(priority_fee) = priority_fee {
            if is_legacy {
                eyre::bail!("there is no priority fee for legacy txs");
            }
            deployer.tx = match deployer.tx {
                TypedTransaction::Eip1559(eip1559_tx_request) => TypedTransaction::Eip1559(
                    eip1559_tx_request.max_priority_fee_per_gas(priority_fee),
                ),
                _ => deployer.tx,
            };
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

            self.verify_preflight_check(constructor_args.clone(), chain).await?;
        }

        // Deploy the actual contract
        let (deployed_contract, receipt) = deployer.send_with_receipt().await?;

        let address = deployed_contract;
        if self.json {
            let output = json!({
                "deployer": deployer_address.to_alloy().to_string(),
                "deployedTo": address.to_string(),
                "transactionHash": receipt.transaction_hash
            });
            println!("{output}");
        } else {
            println!("Deployer: {}", deployer_address.to_alloy());
            println!("Deployed to: {address}");
            println!("Transaction hash: {:?}", receipt.transaction_hash);
        };

        if !self.verify {
            return Ok(())
        }

        println!("Starting contract verification...");

        let num_of_optimizations =
            if self.opts.compiler.optimize { self.opts.compiler.optimizer_runs } else { None };
        let verify = verify::VerifyArgs {
            address,
            contract: self.contract,
            compiler_version: None,
            constructor_args,
            constructor_args_path: None,
            num_of_optimizations,
            etherscan: EtherscanOpts { key: self.eth.etherscan.key, chain: Some(chain.into()) },
            flatten: false,
            force: false,
            watch: true,
            retry: self.retry,
            libraries: vec![],
            root: None,
            verifier: self.verifier,
            show_standard_json_input: false,
        };
        println!("Waiting for {} to detect contract deployment...", verify.verifier.verifier);
        verify.run().await
    }

    fn parse_constructor_args(
        &self,
        constructor: &Constructor,
        constructor_args: &[String],
    ) -> Result<Vec<DynSolValue>> {
        let params = constructor
            .inputs
            .iter()
            .zip(constructor_args)
            .map(|(input, arg)| (DynSolType::parse(&input.ty).expect("Could not parse types"), arg))
            .collect::<Vec<_>>();
        let params_2 = params.iter().map(|(ty, arg)| (ty, arg.as_str())).collect::<Vec<_>>();
        parse_tokens(params_2, true)
    }
}

use ethers::{
    contract::{ContractError, ContractInstance},
    providers::call_raw::{CallBuilder, RawCall},
    types::{
        BlockNumber, Eip1559TransactionRequest, NameOrAddress, TransactionReceipt,
        TransactionRequest,
    },
};

use std::{borrow::Borrow, marker::PhantomData};

/// `ContractDeployer` is a [`ContractDeploymentTx`] object with an
/// [`Arc`] middleware. This type alias exists to preserve backwards
/// compatibility with less-abstract Contracts.
///
/// For full usage docs, see [`ContractDeploymentTx`].
pub type ContractDeployer<M, C> = ContractDeploymentTx<Arc<M>, M, C>;

/// `ContractFactory` is a [`DeploymentTxFactory`] object with an
/// [`Arc`] middleware. This type alias exists to preserve backwards
/// compatibility with less-abstract Contracts.
///
/// For full usage docs, see [`DeploymentTxFactory`].
pub type ContractFactory<M> = DeploymentTxFactory<Arc<M>, M>;

/// Helper which manages the deployment transaction of a smart contract. It
/// wraps a deployment transaction, and retrieves the contract address output
/// by it.
///
/// Currently, we recommend using the [`ContractDeployer`] type alias.
#[derive(Debug)]
#[must_use = "DeploymentTx does nothing unless you `send` it"]
pub struct ContractDeploymentTx<B, M, C> {
    /// the actual deployer, exposed for overriding the defaults
    pub deployer: Deployer<B, M>,
    /// marker for the `Contract` type to create afterwards
    ///
    /// this type will be used to construct it via `From::from(Contract)`
    _contract: PhantomData<C>,
}

impl<B, M, C> Clone for ContractDeploymentTx<B, M, C>
where
    B: Clone,
{
    fn clone(&self) -> Self {
        ContractDeploymentTx { deployer: self.deployer.clone(), _contract: self._contract }
    }
}

impl<B, M, C> From<Deployer<B, M>> for ContractDeploymentTx<B, M, C> {
    fn from(deployer: Deployer<B, M>) -> Self {
        Self { deployer, _contract: PhantomData }
    }
}

impl<B, M, C> ContractDeploymentTx<B, M, C>
where
    B: Borrow<M> + Clone,
    M: Middleware,
    C: From<ContractInstance<B, M>>,
{
    /// Create a new instance of this from a deployer.
    pub fn new(deployer: Deployer<B, M>) -> Self {
        Self { deployer, _contract: PhantomData }
    }

    /// Sets the number of confirmations to wait for the contract deployment transaction
    pub fn confirmations<T: Into<usize>>(mut self, confirmations: T) -> Self {
        self.deployer.confs = confirmations.into();
        self
    }

    /// Sets the block at which RPC requests are made
    pub fn block<T: Into<BlockNumber>>(mut self, block: T) -> Self {
        self.deployer.block = block.into();
        self
    }

    /// Uses a Legacy transaction instead of an EIP-1559 one to do the deployment
    pub fn legacy(mut self) -> Self {
        self.deployer = self.deployer.legacy();
        self
    }

    /// Sets the `from` field in the deploy transaction to the provided value
    pub fn from<T: Into<Address>>(mut self, from: T) -> Self {
        self.deployer.tx.set_from(from.into().to_ethers());
        self
    }

    /// Sets the `to` field in the deploy transaction to the provided value
    pub fn to<T: Into<NameOrAddress>>(mut self, to: T) -> Self {
        self.deployer.tx.set_to(to.into());
        self
    }

    /// Sets the `gas` field in the deploy transaction to the provided value
    pub fn gas<T: Into<U256>>(mut self, gas: T) -> Self {
        self.deployer.tx.set_gas(gas.into().to_ethers());
        self
    }

    /// Sets the `gas_price` field in the deploy transaction to the provided value
    pub fn gas_price<T: Into<U256>>(mut self, gas_price: T) -> Self {
        self.deployer.tx.set_gas_price(gas_price.into().to_ethers());
        self
    }

    /// Sets the `value` field in the deploy transaction to the provided value
    pub fn value<T: Into<U256>>(mut self, value: T) -> Self {
        self.deployer.tx.set_value(value.into().to_ethers());
        self
    }

    /// Sets the `data` field in the deploy transaction to the provided value
    pub fn data<T: Into<Bytes>>(mut self, data: T) -> Self {
        self.deployer.tx.set_data(data.into().0.into());
        self
    }

    /// Sets the `nonce` field in the deploy transaction to the provided value
    pub fn nonce<T: Into<U256>>(mut self, nonce: T) -> Self {
        self.deployer.tx.set_nonce(nonce.into().to_ethers());
        self
    }

    /// Sets the `chain_id` field in the deploy transaction to the provided value
    pub fn chain_id<T: Into<U64>>(mut self, chain_id: T) -> Self {
        self.deployer.tx.set_chain_id(chain_id.into().to_ethers());
        self
    }

    /// Dry runs the deployment of the contract
    ///
    /// Note: this function _does not_ send a transaction from your account
    pub async fn call(&self) -> Result<(), ContractError<M>> {
        self.deployer.call().await
    }

    /// Returns a CallBuilder, which when awaited executes the deployment of this contract via
    /// `eth_call`. This call resolves to the returned data which would have been stored at the
    /// destination address had the deploy transaction been executed via `send()`.
    ///
    /// Note: this function _does not_ send a transaction from your account
    pub fn call_raw(&self) -> CallBuilder<'_, M::Provider> {
        self.deployer.call_raw()
    }

    /// Broadcasts the contract deployment transaction and after waiting for it to
    /// be sufficiently confirmed (default: 1), it returns a new instance of the contract type at
    /// the deployed contract's address.
    pub async fn send(self) -> Result<Address, ContractError<M>> {
        let contract = self.deployer.send().await?;
        Ok(contract)
    }

    /// Broadcasts the contract deployment transaction and after waiting for it to
    /// be sufficiently confirmed (default: 1), it returns a new instance of the contract type at
    /// the deployed contract's address and the corresponding
    /// [`TransactionReceipt`].
    pub async fn send_with_receipt(
        self,
    ) -> Result<(Address, TransactionReceipt), ContractError<M>> {
        let (contract, receipt) = self.deployer.send_with_receipt().await?;
        Ok((contract, receipt))
    }

    /// Returns a reference to the deployer's ABI
    pub fn abi(&self) -> &Abi {
        self.deployer.abi()
    }

    /// Returns a pointer to the deployer's client
    pub fn client(&self) -> &M {
        self.deployer.client()
    }
}

/// Helper which manages the deployment transaction of a smart contract
#[derive(Debug)]
#[must_use = "Deployer does nothing unless you `send` it"]
pub struct Deployer<B, M> {
    /// The deployer's transaction, exposed for overriding the defaults
    pub tx: TypedTransaction,
    abi: Abi,
    client: B,
    confs: usize,
    block: BlockNumber,
    _m: PhantomData<M>,
}

impl<B, M> Clone for Deployer<B, M>
where
    B: Clone,
{
    fn clone(&self) -> Self {
        Deployer {
            tx: self.tx.clone(),
            abi: self.abi.clone(),
            client: self.client.clone(),
            confs: self.confs,
            block: self.block,
            _m: PhantomData,
        }
    }
}

impl<B, M> Deployer<B, M>
where
    B: Borrow<M> + Clone,
    M: Middleware,
{
    /// Sets the number of confirmations to wait for the contract deployment transaction
    pub fn confirmations<T: Into<usize>>(mut self, confirmations: T) -> Self {
        self.confs = confirmations.into();
        self
    }

    /// Set the block at which requests are made
    pub fn block<T: Into<BlockNumber>>(mut self, block: T) -> Self {
        self.block = block.into();
        self
    }

    /// Uses a Legacy transaction instead of an EIP-1559 one to do the deployment
    pub fn legacy(mut self) -> Self {
        self.tx = match self.tx {
            TypedTransaction::Eip1559(inner) => {
                let tx: TransactionRequest = inner.into();
                TypedTransaction::Legacy(tx)
            }
            other => other,
        };
        self
    }

    /// Dry runs the deployment of the contract
    ///
    /// Note: this function _does not_ send a transaction from your account
    pub async fn call(&self) -> Result<(), ContractError<M>> {
        unimplemented!()
    }

    /// Returns a CallBuilder, which when awaited executes the deployment of this contract via
    /// `eth_call`. This call resolves to the returned data which would have been stored at the
    /// destination address had the deploy transaction been executed via `send()`.
    ///
    /// Note: this function _does not_ send a transaction from your account
    pub fn call_raw(&self) -> CallBuilder<'_, M::Provider> {
        self.client.borrow().provider().call_raw(&self.tx).block(self.block.into())
    }

    /// Broadcasts the contract deployment transaction and after waiting for it to
    /// be sufficiently confirmed (default: 1), it returns a [`Contract`](crate::Contract)
    /// struct at the deployed contract's address.
    pub async fn send(self) -> Result<Address, ContractError<M>> {
        let (contract, _) = self.send_with_receipt().await?;
        Ok(contract)
    }

    /// Broadcasts the contract deployment transaction and after waiting for it to
    /// be sufficiently confirmed (default: 1), it returns a tuple with
    /// the [`Contract`](crate::Contract) struct at the deployed contract's address
    /// and the corresponding [`TransactionReceipt`].
    pub async fn send_with_receipt(
        self,
    ) -> Result<(Address, TransactionReceipt), ContractError<M>> {
        let pending_tx = self
            .client
            .borrow()
            .send_transaction(self.tx, Some(self.block.into()))
            .await
            .map_err(ContractError::from_middleware_error)?;

        // TODO: Should this be calculated "optimistically" by address/nonce?
        let receipt = pending_tx
            .confirmations(self.confs)
            .await
            .ok()
            .flatten()
            .ok_or(ContractError::ContractNotDeployed)?;
        let address = receipt.contract_address.ok_or(ContractError::ContractNotDeployed)?;

        Ok((address.to_alloy(), receipt))
    }

    /// Returns a reference to the deployer's ABI
    pub fn abi(&self) -> &Abi {
        &self.abi
    }

    /// Returns a pointer to the deployer's client
    pub fn client(&self) -> &M {
        self.client.borrow()
    }
}

/// To deploy a contract to the Ethereum network, a `ContractFactory` can be
/// created which manages the Contract bytecode and Application Binary Interface
/// (ABI), usually generated from the Solidity compiler.
///
/// Once the factory's deployment transaction is mined with sufficient confirmations,
/// the [`Contract`](crate::Contract) object is returned.
///
/// # Example
///
/// ```no_run
/// use ethers_contract::ContractFactory;
/// use ethers_core::types::Bytes;
/// use ethers_providers::{Provider, Http};
///
/// # async fn foo() -> Result<(), Box<dyn std::error::Error>> {
/// // get the contract ABI and bytecode
/// let abi = Default::default();
/// let bytecode = Bytes::from_static(b"...");
///
/// // connect to the network
/// let client = Provider::<Http>::try_from("http://localhost:8545").unwrap();
/// let client = std::sync::Arc::new(client);
///
/// // create a factory which will be used to deploy instances of the contract
/// let factory = ContractFactory::new(abi, bytecode, client);
///
/// // The deployer created by the `deploy` call exposes a builder which gets consumed
/// // by the async `send` call
/// let contract = factory
///     .deploy("initial value".to_string())?
///     .confirmations(0usize)
///     .send()
///     .await?;
/// println!("{}", contract.address());
/// # Ok(())
/// # }
#[derive(Debug)]
pub struct DeploymentTxFactory<B, M> {
    client: B,
    abi: Abi,
    bytecode: Bytes,
    _m: PhantomData<M>,
}

impl<B, M> Clone for DeploymentTxFactory<B, M>
where
    B: Clone,
{
    fn clone(&self) -> Self {
        DeploymentTxFactory {
            client: self.client.clone(),
            abi: self.abi.clone(),
            bytecode: self.bytecode.clone(),
            _m: PhantomData,
        }
    }
}

impl<B, M> DeploymentTxFactory<B, M>
where
    B: Borrow<M> + Clone,
    M: Middleware,
{
    /// Creates a factory for deployment of the Contract with bytecode, and the
    /// constructor defined in the abi. The client will be used to send any deployment
    /// transaction.
    pub fn new(abi: Abi, bytecode: Bytes, client: B) -> Self {
        Self { client, abi, bytecode, _m: PhantomData }
    }

    /// Create a deployment tx using the provided tokens as constructor
    /// arguments
    pub fn deploy_tokens(self, params: Vec<DynSolValue>) -> Result<Deployer<B, M>, ContractError<M>>
    where
        B: Clone,
    {
        // Encode the constructor args & concatenate with the bytecode if necessary
        let data: Bytes = match (self.abi.constructor(), params.is_empty()) {
            (None, false) => return Err(ContractError::ConstructorError),
            (None, true) => self.bytecode.clone(),
            (Some(constructor), _) => constructor
                .abi_encode_input(&params)
                .map_err(|f| ContractError::DetokenizationError(InvalidOutputType(f.to_string())))?
                .into(),
        };

        // create the tx object. Since we're deploying a contract, `to` is `None`
        // We default to EIP-1559 transactions, but the sender can convert it back
        // to a legacy one
        #[cfg(feature = "legacy")]
        let tx = TransactionRequest { to: None, data: Some(data), ..Default::default() };
        #[cfg(not(feature = "legacy"))]
        let tx =
            Eip1559TransactionRequest { to: None, data: Some(data.0.into()), ..Default::default() };
        let tx = tx.into();

        Ok(Deployer {
            client: self.client.clone(),
            abi: self.abi,
            tx,
            confs: 1,
            block: BlockNumber::Latest,
            _m: PhantomData,
        })
    }

    /// Constructs the deployment transaction based on the provided constructor
    /// arguments and returns a `Deployer` instance. You must call `send()` in order
    /// to actually deploy the contract.
    ///
    /// Notes:
    /// 1. If there are no constructor arguments, you should pass `()` as the argument.
    /// 1. The default poll duration is 7 seconds.
    /// 1. The default number of confirmations is 1 block.
    pub fn deploy(
        self,
        constructor_args: Vec<DynSolValue>,
    ) -> Result<Deployer<B, M>, ContractError<M>> {
        self.deploy_tokens(constructor_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
