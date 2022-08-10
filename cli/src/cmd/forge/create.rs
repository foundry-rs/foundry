//! Create command
use super::verify;
use crate::{
    cmd::{forge::build::CoreBuildArgs, utils, LoadConfig, RetryArgs},
    compile,
    opts::{EthereumOpts, TransactionOpts, WalletType},
};
use cast::SimpleCast;
use clap::{Parser, ValueHint};
use ethers::{
    abi::{Abi, Constructor, Token},
    prelude::{artifacts::BytecodeObject, ContractFactory, Middleware},
    solc::{
        info::ContractInfo,
        utils::{canonicalized, read_json_file},
    },
    types::{transaction::eip2718::TypedTransaction, Chain},
};
use eyre::Context;
use foundry_common::{fs, get_http_provider};
use foundry_utils::parse_tokens;
use rustc_hex::ToHex;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tracing::log::trace;

pub const RETRY_VERIFY_ON_CREATE: RetryArgs = RetryArgs { retries: 15, delay: Some(3) };

#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    #[clap(
        help = "The contract identifier in the form `<path>:<contractname>`.",
        value_name = "CONTRACT"
    )]
    contract: ContractInfo,

    #[clap(
        long,
        multiple_values = true,
        help = "The constructor arguments.",
        name = "constructor_args",
        conflicts_with = "constructor_args_path",
        value_name = "ARGS"
    )]
    constructor_args: Vec<String>,

    #[clap(
        long,
        help = "The path to a file containing the constructor arguments.",
        value_hint = ValueHint::FilePath,
        name = "constructor_args_path",
        conflicts_with = "constructor_args",
        value_name = "FILE"
    )]
    constructor_args_path: Option<PathBuf>,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    opts: CoreBuildArgs,

    #[clap(flatten, next_help_heading = "TRANSACTION OPTIONS")]
    tx: TransactionOpts,

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

    #[clap(
        long,
        help = "Send via `eth_sendTransaction` using the `--from` argument or `$ETH_FROM` as sender",
        requires = "from"
    )]
    unlocked: bool,
}

impl CreateArgs {
    /// Executes the command to create a contract
    pub async fn run(mut self) -> eyre::Result<()> {
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

        let (abi, bin, _) = utils::remove_contract(&mut output, &self.contract)?;

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
        let config = self.eth.load_config_emit_warnings();
        let provider = Arc::new(get_http_provider(
            config.eth_rpc_url.as_deref().unwrap_or("http://localhost:8545"),
        ));
        let params = match abi.constructor {
            Some(ref v) => {
                let constructor_args =
                    if let Some(ref constructor_args_path) = self.constructor_args_path {
                        if !constructor_args_path.exists() {
                            eyre::bail!(
                                "Constructor args file \"{}\" not found",
                                constructor_args_path.display()
                            );
                        }
                        if constructor_args_path.extension() == Some(std::ffi::OsStr::new("json")) {
                            match read_json_file(constructor_args_path) {
                                Ok(args) => args,
                                Err(err) => eyre::bail!(
                                    "Constructor args file \"{}\" must encode a json array: \"{}\"",
                                    constructor_args_path.display(),
                                    err
                                ),
                            }
                        } else {
                            let file = fs::read_to_string(constructor_args_path)?;
                            file.split_whitespace().map(str::to_string).collect::<Vec<String>>()
                        }
                    } else {
                        self.constructor_args.clone()
                    };
                self.parse_constructor_args(v, &constructor_args)?
            }
            None => vec![],
        };

        if self.unlocked {
            let sender = self.eth.wallet.from.expect("is required");
            trace!("creating with unlocked account={:?}", sender);
            // use unlocked provider
            let provider =
                Arc::try_unwrap(provider).expect("Only one ref; qed.").with_sender(sender);
            self.deploy(abi, bin, params, provider).await?;
            return Ok(())
        }

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
    ) -> eyre::Result<()> {
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

        // set gas price if specified
        if let Some(gas_price) = self.tx.gas_price {
            deployer.tx.set_gas_price(gas_price);
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
        if let Some(priority_fee) = self.tx.priority_gas_price {
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

        let (deployed_contract, receipt) = deployer.send_with_receipt().await?;

        let address = deployed_contract.address();
        if self.json {
            let output = json!({
                "deployer": SimpleCast::checksum_address(&deployer_address)?,
                "deployedTo": SimpleCast::checksum_address(&address)?,
                "transactionHash": receipt.transaction_hash
            });
            println!("{output}");
        } else {
            println!("Deployer: {}", SimpleCast::checksum_address(&deployer_address)?);
            println!("Deployed to: {}", SimpleCast::checksum_address(&address)?);
            println!("Transaction hash: {:?}", receipt.transaction_hash);
        };

        if !self.verify {
            return Ok(())
        }

        println!("Starting contract verification...");
        let constructor_args = if !args.is_empty() {
            // we're passing an empty vec to the `encode_input` of the constructor because we only
            // need the constructor arguments and the encoded input is `code + args`
            let code = Vec::new();
            let encoded_args = abi
                .constructor()
                .ok_or(eyre::eyre!("could not find constructor"))?
                .encode_input(code, &args)?
                .to_hex::<String>();
            Some(encoded_args)
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
            libraries: vec![],
            root: None,
        };
        println!("Waiting for etherscan to detect contract deployment...");
        verify.run().await
    }

    fn parse_constructor_args(
        &self,
        constructor: &Constructor,
        constructor_args: &[String],
    ) -> eyre::Result<Vec<Token>> {
        let params = constructor
            .inputs
            .iter()
            .zip(constructor_args)
            .map(|(input, arg)| (&input.kind, arg.as_str()))
            .collect::<Vec<_>>();

        parse_tokens(params, true)
    }
}
