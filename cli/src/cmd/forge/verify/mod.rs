//! Verify contract source

use crate::{
    cmd::{
        forge::verify::{etherscan::EtherscanVerificationProvider, provider::VerificationProvider},
        retry::RetryArgs,
        LoadConfig,
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
    /// The contract verification provider to use.
    #[clap(long, help_heading = "Verifier options", default_value = "etherscan", value_enum)]
    pub verifier: VerificationProviderType,

    /// The verifier URL, if using a custom provider
    #[clap(long, help_heading = "Verifier options", env = "VERIFIER_URL")]
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
    /// The address of the contract to verify.
    pub address: Address,

    /// The contract identifier in the form `<path>:<contractname>`.
    pub contract: ContractInfo,

    /// The ABI-encoded constructor arguments.
    #[clap(long, conflicts_with = "constructor_args_path", value_name = "ARGS")]
    pub constructor_args: Option<String>,

    /// The path to a file containing the constructor arguments.
    #[clap(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub constructor_args_path: Option<PathBuf>,

    /// The `solc` version to use to build the smart contract.
    #[clap(long, value_name = "VERSION")]
    pub compiler_version: Option<String>,

    /// The number of optimization runs used to build the smart contract.
    #[clap(long, visible_alias = "optimizer-runs", value_name = "NUM")]
    pub num_of_optimizations: Option<usize>,

    /// Flatten the source code before verifying.
    #[clap(long)]
    pub flatten: bool,

    /// Do not compile the flattened smart contract before verifying (if --flatten is passed).
    #[clap(short, long)]
    pub force: bool,

    /// Wait for verification result after submission.
    #[clap(long)]
    pub watch: bool,

    /// Set pre-linked libraries.
    #[clap(long, help_heading = "Linker options", env = "DAPP_LIBRARIES")]
    pub libraries: Vec<String>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Prints the standard json compiler input.
    ///
    /// The standard json compiler input can be used to manually submit contract verification in
    /// the browser.
    #[clap(long, conflicts_with = "flatten")]
    pub show_standard_json_input: bool,

    #[clap(flatten)]
    pub etherscan: EtherscanOpts,

    #[clap(flatten)]
    pub retry: RetryArgs,

    #[clap(flatten)]
    pub verifier: VerifierArgs,
}

impl_figment_convert!(VerifyArgs);

impl figment::Provider for VerifyArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Provider")
    }
    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        let mut dict = self.etherscan.dict();
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
        let config = self.load_config_emit_warnings();
        let chain = config.chain_id.unwrap_or_default();
        self.etherscan.chain = Some(chain);
        self.etherscan.key = config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);

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
    /// The verification ID.
    ///
    /// For Etherscan - Submission GUID.
    ///
    /// For Sourcify - Contract Address.
    id: String,

    #[clap(flatten)]
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
