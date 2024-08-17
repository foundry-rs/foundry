//! Errors for this crate

use foundry_config::Chain;
use std::fmt;

/// An error thrown when resolving a function via signature failed
#[derive(Clone, Debug)]
pub enum FunctionSignatureError {
    MissingSignature,
    MissingEtherscan { sig: String },
    UnknownChain(Chain),
    MissingToAddress,
}

impl fmt::Display for FunctionSignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSignature => {
                writeln!(f, "Function signature must be set")
            }
            Self::MissingEtherscan { sig } => {
                writeln!(f, "Failed to determine function signature for `{sig}`")?;
                writeln!(f, "To lookup a function signature of a deployed contract by name, a valid ETHERSCAN_API_KEY must be set.")?;
                write!(f, "\tOr did you mean:\t {sig}()")
            }
            Self::UnknownChain(chain) => {
                write!(f, "Resolving via etherscan requires a known chain. Unknown chain: {chain}")
            }
            Self::MissingToAddress => f.write_str("Target address must be set"),
        }
    }
}

impl std::error::Error for FunctionSignatureError {}
