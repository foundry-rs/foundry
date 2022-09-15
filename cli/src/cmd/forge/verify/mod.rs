//! Verify contract source

use crate::cmd::retry::RetryArgs;
use clap::{Parser, ValueHint};
use ethers::{abi::Address, solc::info::ContractInfo};
use foundry_config::{figment, impl_figment_convert, Chain, Config};
use provider::VerificationProviderType;
use std::path::PathBuf;

mod etherscan;
pub mod provider;
mod sourcify;

/// Verification provider arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifierArgs {
    #[clap(
        arg_enum,
        long = "verifier",
        help_heading = "Verification Provider",
        help = "Contract verification provider to use `etherscan`, `sourcify` or `blockscout`",
        default_value = "etherscan"
    )]
    pub verifier: VerificationProviderType,

    #[clap(
        long,
        env = "VERIFIER_URL",
        help = "The verifier URL, if using a custom provider",
        value_name = "VERIFIER_URL"
    )]
    verifier_url: Option<String>,
}

impl Default for VerifierArgs {
    fn default() -> Self {
        VerifierArgs { verifier: VerificationProviderType::Etherscan, verifier_url: None }
    }
}

/// Verification arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    #[clap(help = "The address of the contract to verify.", value_name = "ADDRESS")]
    pub address: Address,

    #[clap(
        help = "The contract identifier in the form `<path>:<contractname>`.",
        value_name = "CONTRACT"
    )]
    pub contract: ContractInfo,

    #[clap(
        long,
        help = "The ABI-encoded constructor arguments.",
        name = "constructor_args",
        conflicts_with = "constructor_args_path",
        value_name = "ARGS"
    )]
    pub constructor_args: Option<String>,

    #[clap(
        long,
        help = "The path to a file containing the constructor arguments.",
        value_hint = ValueHint::FilePath,
        name = "constructor_args_path",
        conflicts_with = "constructor_args",
        value_name = "FILE"
    )]
    pub constructor_args_path: Option<PathBuf>,

    #[clap(
        long,
        help = "The compiler version used to build the smart contract.",
        value_name = "VERSION"
    )]
    pub compiler_version: Option<String>,

    #[clap(
        visible_alias = "optimizer-runs",
        long,
        help = "The number of optimization runs used to build the smart contract.",
        value_name = "NUM"
    )]
    pub num_of_optimizations: Option<usize>,

    #[clap(
        long,
        visible_alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet",
        value_name = "CHAIN"
    )]
    pub chain: Chain,

    #[clap(
        help = "Your Etherscan API key.",
        env = "ETHERSCAN_API_KEY",
        value_name = "ETHERSCAN_KEY"
    )]
    pub etherscan_key: Option<String>,

    #[clap(help = "Flatten the source code before verifying.", long = "flatten")]
    pub flatten: bool,

    #[clap(
        short,
        long,
        help = "Do not compile the flattened smart contract before verifying (if --flatten is passed)."
    )]
    pub force: bool,

    #[clap(long, help = "Wait for verification result after submission")]
    pub watch: bool,

    #[clap(flatten, help = "Allows to use retry arguments for contract verification")]
    pub retry: RetryArgs,

    #[clap(
        help_heading = "LINKER OPTIONS",
        help = "Set pre-linked libraries.",
        long,
        env = "DAPP_LIBRARIES",
        value_name = "LIBRARIES"
    )]
    pub libraries: Vec<String>,

    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    pub root: Option<PathBuf>,

    #[clap(flatten)]
    pub verifier: VerifierArgs,
}

impl_figment_convert!(VerifyArgs);

impl figment::Provider for VerifyArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named(stringify!($name))
    }
    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        let mut dict = figment::value::Dict::new();
        if let Some(root) = self.root.as_ref() {
            dict.insert("root".to_string(), figment::value::Value::serialize(root)?);
        }
        if let Some(optimizer_runs) = self.num_of_optimizations {
            dict.insert("optimizer".to_string(), figment::value::Value::serialize(true)?);
            dict.insert(
                "optimizer_runs".to_string(),
                figment::value::Value::serialize(optimizer_runs)?,
            );
        }
        Ok(figment::value::Map::from([(Config::selected_profile(), dict)]))
    }
}

impl VerifyArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(self) -> eyre::Result<()> {
        self.verifier.verifier.client(&self.etherscan_key)?.verify(self).await
    }
}

/// Check verification status arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyCheckArgs {
    #[clap(
        help = "The verification ID. For Etherscan - Submission GUID. For Sourcify - Contract Address",
        value_name = "ID"
    )]
    id: String,

    #[clap(
        long,
        visible_alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet",
        value_name = "CHAIN"
    )]
    chain: Chain,

    #[clap(flatten, help = "Allows to use retry arguments for contract verification")]
    retry: RetryArgs,

    #[clap(
        long,
        help = "Your Etherscan API key.",
        env = "ETHERSCAN_API_KEY",
        value_name = "ETHERSCAN_KEY"
    )]
    etherscan_key: Option<String>,

    #[clap(flatten)]
    verifier: VerifierArgs,
}

impl VerifyCheckArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(self) -> eyre::Result<()> {
        self.verifier.verifier.client(&self.etherscan_key)?.check(self).await
    }
}
