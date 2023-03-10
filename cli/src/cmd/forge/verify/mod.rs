//! Verify contract source

use crate::{
    cmd::{
        forge::verify::{etherscan::EtherscanVerificationProvider, provider::VerificationProvider},
        retry::RetryArgs,
    },
    opts::EtherscanOpts,
};
use clap::{Parser, ValueHint};
use ethers::{abi::Address, solc::info::ContractInfo};
use foundry_config::{figment, impl_figment_convert, impl_figment_convert_cast, Config};
use provider::VerificationProviderType;
use reqwest::Url;
use std::path::PathBuf;

mod etherscan;
pub mod provider;
mod sourcify;

/// Verification provider arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifierArgs {
    #[clap(
        value_enum,
        long = "verifier",
        help_heading = "Verification provider",
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
    pub verifier_url: Option<String>,
}

impl Default for VerifierArgs {
    fn default() -> Self {
        VerifierArgs { verifier: VerificationProviderType::Etherscan, verifier_url: None }
    }
}

/// CLI arguments for `forge verify`.
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

    #[clap(flatten)]
    pub etherscan: EtherscanOpts,

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

    #[clap(flatten)]
    pub retry: RetryArgs,

    #[clap(
        help_heading = "Linker options",
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

    #[clap(
        help = "Prints the standard json compiler input.",
        long,
        long_help = "The standard json compiler input can be used to manually submit contract verification in the browser.",
        conflicts_with = "flatten"
    )]
    pub show_standard_json_input: bool,
}

impl_figment_convert!(VerifyArgs);

impl figment::Provider for VerifyArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Provider")
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
    pub async fn run(mut self) -> eyre::Result<()> {
        let config = Config::from(&self.etherscan);
        let chain = config.chain_id.unwrap_or_default();
        self.etherscan.chain = Some(chain);
        self.etherscan.key = config.get_etherscan_api_key(Some(chain));

        if self.show_standard_json_input {
            let args =
                EtherscanVerificationProvider::default().create_verify_request(&self, None).await?;
            println!("{}", args.source);
            return Ok(())
        }

        let verifier_url = self.verifier.verifier_url.clone();
        println!("Start verifying contract `{:?}` deployed on {chain}", self.address);
        self.verifier.verifier.client(&self.etherscan.key)?.verify(self).await.map_err(|err| {
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

    /// Returns the configured verification provider
    pub fn verification_provider(&self) -> eyre::Result<Box<dyn VerificationProvider>> {
        self.verifier.verifier.client(&self.etherscan.key)
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

    #[clap(flatten, help = "Allows to use retry arguments for contract verification")]
    retry: RetryArgs,

    #[clap(flatten)]
    etherscan: EtherscanOpts,

    #[clap(flatten)]
    verifier: VerifierArgs,
}

impl_figment_convert_cast!(VerifyCheckArgs);

impl VerifyCheckArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(self) -> eyre::Result<()> {
        println!("Checking verification status on {}", self.etherscan.chain.unwrap_or_default());
        self.verifier.verifier.client(&self.etherscan.key)?.check(self).await
    }
}

impl figment::Provider for VerifyCheckArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Check Provider")
    }

    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        self.etherscan.data()
    }
}

/// Returns `true` if the URL only consists of host.
///
/// This is used to check user input url for missing /api path
#[inline]
fn is_host_only(url: &Url) -> bool {
    matches!(url.path(), "/" | "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_only() {
        assert!(!is_host_only(&Url::parse("https://blockscout.net/api").unwrap()));
        assert!(is_host_only(&Url::parse("https://blockscout.net/").unwrap()));
        assert!(is_host_only(&Url::parse("https://blockscout.net").unwrap()));
    }
}
