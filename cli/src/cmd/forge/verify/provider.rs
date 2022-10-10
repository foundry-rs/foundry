use crate::cmd::forge::verify::{
    etherscan::EtherscanVerificationProvider, sourcify::SourcifyVerificationProvider, VerifyArgs,
    VerifyCheckArgs,
};
use async_trait::async_trait;
use std::{fmt, str::FromStr};

/// An abstraction for various verification providers such as etherscan, sourcify, blockscout
#[async_trait]
pub trait VerificationProvider {
    async fn verify(&self, args: VerifyArgs) -> eyre::Result<()>;
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
    #[allow(clippy::box_default)]
    pub fn client(&self, key: &Option<String>) -> eyre::Result<Box<dyn VerificationProvider>> {
        match self {
            VerificationProviderType::Etherscan => {
                if key.as_ref().map_or(true, |key| key.is_empty()) {
                    eyre::bail!("ETHERSCAN_API_KEY must be set")
                }
                Ok(Box::new(EtherscanVerificationProvider::default()))
            }
            VerificationProviderType::Sourcify => {
                Ok(Box::new(SourcifyVerificationProvider::default()))
            }
            VerificationProviderType::Blockscout => {
                Ok(Box::new(EtherscanVerificationProvider::default()))
            }
        }
    }
}
