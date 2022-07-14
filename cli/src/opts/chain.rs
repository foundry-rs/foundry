use clap::Parser;
use ethers::types::Chain;
use strum::VariantNames;

// Helper for exposing enum values for `Chain`
// TODO: Is this a duplicate of config/src/chain.rs?
#[derive(Debug, Clone, Parser)]
pub struct ClapChain {
    #[clap(
        short = 'c',
        long = "chain",
        env = "CHAIN",
        default_value = "mainnet",
        // if Chain implemented ArgEnum, we'd get this for free
        possible_values = Chain::VARIANTS,
        value_name = "CHAIN"
    )]
    pub inner: Chain,
}
