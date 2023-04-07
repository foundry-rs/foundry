use crate::cmd::forge::verify::{
    etherscan::EtherscanVerificationProvider, sourcify::SourcifyVerificationProvider, VerifyArgs,
    VerifyCheckArgs,
};
use async_trait::async_trait;
use std::{fmt, str::FromStr};

/// An abstraction for various verification providers such as etherscan, sourcify, blockscout
#[async_trait]
pub trait VerificationProvider {
    /// This should ensure the verify request can be prepared successfully.
    ///
    /// Caution: Implementers must ensure that this _never_ sends the actual verify request
    /// `[VerificationProvider::verify]`, instead this is supposed to evaluate whether the given
    /// [`VerifyArgs`] are valid to begin with. This should prevent situations where there's a
    /// contract deployment that's executed before the verify request and the subsequent verify task
    /// fails due to misconfiguration.
    async fn preflight_check(&mut self, args: VerifyArgs) -> eyre::Result<()>;

    /// Sends the actual verify request for the targeted contract.
    async fn verify(&mut self, args: VerifyArgs) -> eyre::Result<()>;

    /// Checks whether the contract is verified.
    async fn check(&self, args: VerifyCheckArgs) -> eyre::Result<()>;
}

impl FromStr for VerificationProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "e" | "etherscan" => Ok(VerificationProviderType::Etherscan),
            "s" | "sourcify" => Ok(VerificationProviderType::Sourcify),
            "b" | "blockscout" => Ok(VerificationProviderType::Blockscout),
            _ => Err(format!("Unknown provider: {s}")),
        }
    }
}

impl fmt::Display for VerificationProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationProviderType::Etherscan => {
                write!(f, "etherscan")?;
            }
            VerificationProviderType::Sourcify => {
                write!(f, "sourcify")?;
            }
            VerificationProviderType::Blockscout => {
                write!(f, "blockscout")?;
            }
        };
        Ok(())
    }
}

#[derive(clap::ValueEnum, Debug, Clone, PartialEq, Eq)]
pub enum VerificationProviderType {
    Etherscan,
    Sourcify,
    Blockscout,
}

impl VerificationProviderType {
    /// Returns the corresponding `VerificationProvider` for the key
    pub fn client(&self, key: &Option<String>) -> eyre::Result<Box<dyn VerificationProvider>> {
        match self {
            VerificationProviderType::Etherscan => {
                if key.as_ref().map_or(true, |key| key.is_empty()) {
                    eyre::bail!("ETHERSCAN_API_KEY must be set")
                }
                Ok(Box::<EtherscanVerificationProvider>::default())
            }
            VerificationProviderType::Sourcify => {
                Ok(Box::<SourcifyVerificationProvider>::default())
            }
            VerificationProviderType::Blockscout => {
                Ok(Box::<EtherscanVerificationProvider>::default())
            }
        }
    }
}
