//! The `forge verify-bytecode` command.

use crate::{utils::is_host_only, verify::VerifierArgs};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{hex, Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, LoadConfig},
};
use foundry_common::{abi::encode_args, compile::ProjectCompiler, provider::ProviderBuilder};
use foundry_compilers::{
    artifacts::{BytecodeHash, CompactContractBytecode, EvmVersion},
    info::ContractInfo,
};
use foundry_config::{figment, filter::SkipBuildFilter, impl_figment_convert, Chain, Config};
use foundry_evm::{
    constants::DEFAULT_CREATE2_DEPLOYER, executors::TracingExecutor, utils::configure_tx_env,
};
use reqwest::Url;
use revm_primitives::{db::Database, EnvWithHandlerCfg, HandlerCfg, SpecId};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, str::FromStr};
use yansi::Paint;

impl_figment_convert!(VerifyBytecodeArgs);

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
    #[clap(short = 'r', long, value_name = "RPC_URL", env = "ETH_RPC_URL")]
    pub rpc_url: Option<String>,

    #[clap(flatten)]
    pub etherscan: EtherscanOpts,

    /// Verifier options.
    #[clap(flatten)]
    pub verifier: VerifierArgs,

    /// Suppress logs and emit json results to stdout
    #[clap(long, default_value = "false")]
    pub json: bool,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,
}

impl figment::Provider for VerifyBytecodeArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Bytecode Provider")
    }

    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        let mut dict = self.etherscan.dict();
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

        // Get the bytecode at the address, bailing if it doesn't exist.
        let code = provider.get_code_at(self.address).await?;
        if code.is_empty() {
            eyre::bail!("No bytecode found at address {}", self.address);
        }

        if !self.json {
            println!(
                "Verifying bytecode for contract {} at address {}",
                self.contract.name.clone().green(),
                self.address.green()
            );
        }

        // If chain is not set, we try to get it from the RPC.
        // If RPC is not set, the default chain is used.
        let chain = match config.get_rpc_url() {
            Some(_) => utils::get_chain(config.chain, provider).await?,
            None => config.chain.unwrap_or_default(),
        };

        // Set Etherscan options.
        self.etherscan.chain = Some(chain);
        self.etherscan.key = config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);

        let verifier_url = self.verifier.verifier_url.clone();
        self.verifier
            .verifier
            .client(&self.etherscan.key())?
            .verify_bytecode(self)
            .await
            .map_err(|err| {
                if let Some(verifier_url) = verifier_url {
                    match Url::parse(&verifier_url) {
                       Ok(url) => {
                           if is_host_only(&url) {
                               return err.wrap_err(format!(
                                   "Provided URL `{verifier_url}` is host only.\n Did you mean to use the API endpoint`{verifier_url}/api` ?"
                               ))
                           }
                       }
                       Err(url_err) => {
                           return err.wrap_err(format!(
                               "Invalid URL {verifier_url} provided: {url_err}"
                           ))
                       }
                   }
               }

               err
            })
    }
}
