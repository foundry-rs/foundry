use clap::{builder::PossibleValuesParser, Parser};
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
        value_parser = PossibleValuesParser::from(Chain::VARIANTS),
        value_name = "CHAIN"
    )]
    pub inner: Chain,
}
