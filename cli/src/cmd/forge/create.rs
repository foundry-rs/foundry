//! Create command
use super::verify;
use crate::{
    cmd::{forge::build::CoreBuildArgs, Cmd, RetryArgs},
    compile,
    opts::{forge::ContractInfo, EthereumOpts, WalletType},
    utils::{parse_ether_value, parse_u256},
};
use clap::{Parser, ValueHint};
use ethers::{
    abi::{Abi, Constructor, Token},
    prelude::{artifacts::BytecodeObject, ContractFactory, Http, Middleware, Provider},
    solc::utils::RuntimeOrHandle,
    types::{transaction::eip2718::TypedTransaction, Chain, U256},
};
use eyre::{Context, Result};
use foundry_config::Config;
use foundry_utils::parse_tokens;
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

pub const RETRY_VERIFY_ON_CREATE: RetryArgs = RetryArgs { retries: 15, delay: Some(3) };

#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    #[clap(help = "The contract identifier in the form `<path>:<contractname>`.")]
    contract: ContractInfo,

    #[clap(
        long,
        multiple_values = true,
        help = "The constructor arguments.",
        name = "constructor_args",
        conflicts_with = "constructor_args_path"
    )]
    constructor_args: Vec<String>,

    #[clap(
        long,
        help = "The path to a file containing the constructor arguments.",
        value_hint = ValueHint::FilePath,
        name = "constructor_args_path",
        conflicts_with = "constructor_args",
    )]
    constructor_args_path: Option<PathBuf>,

    #[clap(
        long,
        help_heading = "TRANSACTION OPTIONS",
        help = "Send a legacy transaction instead of an EIP1559 transaction.",
        long_help = r#"Send a legacy transaction instead of an EIP1559 transaction.

This is automatically enabled for common networks without EIP1559."#
    )]
    legacy: bool,

    #[clap(
        long = "gas-price",
        help_heading = "TRANSACTION OPTIONS",
        help = "Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.",
        env = "ETH_GAS_PRICE",
        parse(try_from_str = parse_ether_value)
    )]
    gas_price: Option<U256>,

    #[clap(
        long = "gas-limit",
        help_heading = "TRANSACTION OPTIONS",
        help = "Gas limit for the transaction.",
        env = "ETH_GAS_LIMIT",
        parse(try_from_str = parse_u256)
    )]
    gas_limit: Option<U256>,

    #[clap(
        long = "priority-fee", 
        help_heading = "TRANSACTION OPTIONS",
        help = "Gas priority fee for EIP1559 transactions.",
        env = "ETH_GAS_PRIORITY_FEE", parse(try_from_str = parse_ether_value)
    )]
    priority_fee: Option<U256>,
    #[clap(
        long,
        help_heading = "TRANSACTION OPTIONS",
        help = "Ether to send in the transaction.",
        long_help = r#"Ether to send in the transaction, either specified in wei, or as a string with a unit type.

Examples: 1ether, 10gwei, 0.01ether"#,
        parse(try_from_str = parse_ether_value)
    )]
    value: Option<U256>,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    opts: CoreBuildArgs,

    #[clap(flatten, next_help_heading = "ETHEREUM OPTIONS")]
    eth: EthereumOpts,

    #[clap(
        long = "json",
        help_heading = "DISPLAY OPTIONS",
        help = "Print the deployment information as JSON."
    )]
    json: bool,

    #[clap(long, help = "Verify contract after creation.")]
    verify: bool,
}

impl Cmd for CreateArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        RuntimeOrHandle::new().block_on(self.run_create())
    }
}

impl CreateArgs {
    pub async fn run_create(self) -> Result<()> {
        // Find Project & Compile
        let project = self.opts.project()?;
        if self.json {
            // Suppress compile stdout messages when printing json output
            compile::suppress_compile(&project)?;
        } else {
            compile::compile(&project, false, false)?;
        }

        // Get ABI and BIN
        let (abi, bin, _) = crate::cmd::utils::read_artifact(&project, self.contract.clone())?;

        let bin = match bin.object {
            BytecodeObject::Bytecode(_) => bin.object,
            _ => {
                let link_refs = bin
                    .link_references
                    .iter()
                    .flat_map(|(path, names)| {
                        names.keys().map(move |name| format!("\t{}: {}", name, path))
                    })
                    .collect::<Vec<String>>()
                    .join("\n");
                eyre::bail!("Dynamic linking not supported in `create` command - deploy the following library contracts first, then provide the address to link at compile time\n{}", link_refs)
            }
        };

        // Add arguments to constructor
        let config = Config::from(&self.eth);
        let provider = Provider::<Http>::try_from(
            config.eth_rpc_url.unwrap_or_else(|| "http://localhost:8545".to_string()),
        )?;
        let params = match abi.constructor {
            Some(ref v) => {
                let constructor_args =
                    if let Some(ref constructor_args_path) = self.constructor_args_path {
                        if !std::path::Path::new(&constructor_args_path).exists() {
                            eyre::bail!("constructor args path not found");
                        }
                        let file = fs::read_to_string(constructor_args_path)?;
                        file.split(' ').map(|s| s.to_string()).collect::<Vec<String>>()
                    } else {
                        self.constructor_args.clone()
                    };
                self.parse_constructor_args(v, &constructor_args)?
            }
            None => vec![],
        };

        // Deploy with signer
        let chain_id = provider.get_chainid().await?;
        match self.eth.signer_with(chain_id, provider).await? {
            Some(signer) => match signer {
                WalletType::Ledger(signer) => self.deploy(abi, bin, params, signer).await?,
                WalletType::Local(signer) => self.deploy(abi, bin, params, signer).await?,
                WalletType::Trezor(signer) => self.deploy(abi, bin, params, signer).await?,
            },
            None => eyre::bail!("could not find artifact"),
        };

        Ok(())
    }

    async fn deploy<M: Middleware + 'static>(
        self,
        abi: Abi,
        bin: BytecodeObject,
        args: Vec<Token>,
        provider: M,
    ) -> Result<()> {
        let chain = provider.get_chainid().await?.as_u64();
        let deployer_address =
            provider.default_sender().expect("no sender address set for provider");
        let bin = bin.into_bytes().unwrap_or_else(|| {
            panic!("no bytecode found in bin object for {}", self.contract.name)
        });
        let provider = Arc::new(provider);
        let factory = ContractFactory::new(abi.clone(), bin.clone(), provider.clone());

        let is_args_empty = args.is_empty();
        let deployer =
            factory.deploy_tokens(args.clone()).context("Failed to deploy contract").map_err(|e| {
                if is_args_empty {
                    e.wrap_err("No arguments provided for contract constructor. Consider --constructor-args or --constructor-args-path")
                } else {
                    e
                }
            })?;
        let is_legacy =
            self.legacy || Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default();
        let mut deployer = if is_legacy { deployer.legacy() } else { deployer };

        // fill tx first because if you target a lower gas than current base, eth_estimateGas
        // will fail and create will fail
        let mut tx = deployer.tx;
        provider.fill_transaction(&mut tx, None).await?;
        deployer.tx = tx;

        // set gas price if specified
        if let Some(gas_price) = self.gas_price {
            deployer.tx.set_gas_price(gas_price);
        }

        // set gas limit if specified
        if let Some(gas_limit) = self.gas_limit {
            deployer.tx.set_gas(gas_limit);
        }

        // set priority fee if specified
        if let Some(priority_fee) = self.priority_fee {
            if is_legacy {
                panic!("there is no priority fee for legacy txs");
            }
            deployer.tx = match deployer.tx {
                TypedTransaction::Eip1559(eip1559_tx_request) => TypedTransaction::Eip1559(
                    eip1559_tx_request.max_priority_fee_per_gas(priority_fee),
                ),
                _ => deployer.tx,
            };
        }

        // set tx value if specified
        if let Some(value) = self.value {
            deployer.tx.set_value(value);
        }

        let (deployed_contract, receipt) = deployer.send_with_receipt().await?;
        let address = deployed_contract.address();
        if self.json {
            let output = json!({
                "deployer": deployer_address,
                "deployedTo": address,
                "transactionHash": receipt.transaction_hash
            });
            println!("{output}");
        } else {
            println!("Deployer: {deployer_address:?}");
            println!("Deployed to: {:?}", address);
            println!("Transaction hash: {:?}", receipt.transaction_hash);
        };

        if !self.verify {
            return Ok(())
        }

        println!("Starting contract verification...");
        let constructor_args = if !args.is_empty() {
            let encoded_args = abi
                .constructor()
                .ok_or(eyre::eyre!("could not find constructor"))?
                .encode_input(bin.to_vec(), &args)?;
            Some(String::from_utf8(encoded_args)?)
        } else {
            None
        };
        let num_of_optimizations =
            if self.opts.compiler.optimize { self.opts.compiler.optimizer_runs } else { None };
        let verify = verify::VerifyArgs {
            address,
            contract: self.contract,
            compiler_version: None,
            constructor_args,
            num_of_optimizations,
            chain: chain.into(),
            etherscan_key: self
                .eth
                .etherscan_api_key
                .ok_or(eyre::eyre!("ETHERSCAN_API_KEY must be set"))?,
            project_paths: self.opts.project_paths,
            flatten: false,
            force: false,
            watch: true,
            retry: RETRY_VERIFY_ON_CREATE,
        };
        println!("Waiting for etherscan to detect contract deployment...");
        verify.run().await
    }

    fn parse_constructor_args(
        &self,
        constructor: &Constructor,
        constructor_args: &[String],
    ) -> Result<Vec<Token>> {
        let params = constructor
            .inputs
            .iter()
            .zip(constructor_args)
            .map(|(input, arg)| (&input.kind, arg.as_str()))
            .collect::<Vec<_>>();

        parse_tokens(params, true)
    }
}
