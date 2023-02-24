use clap::{
    builder::{PossibleValuesParser, TypedValueParser},
    Parser,
};
use ethers::types::Chain;
use std::ffi::OsStr;
use strum::VariantNames;

// Helper for exposing enum values for `Chain`
// TODO: Is this a duplicate of config/src/chain.rs?
#[derive(Debug, Clone, Parser, Copy)]
pub struct ClapChain {
    #[clap(
        short = 'c',
        long = "chain",
        env = "CHAIN",
        default_value = "mainnet",
        value_parser = ChainValueParser::default(),
        value_name = "CHAIN"
    )]
    pub inner: Chain,
}

/// The value parser for `Chain`s
#[derive(Clone, Debug)]
pub struct ChainValueParser {
    pub inner: PossibleValuesParser,
}

impl Default for ChainValueParser {
    fn default() -> Self {
        ChainValueParser { inner: Chain::VARIANTS.into() }
    }
}

impl TypedValueParser for ChainValueParser {
    type Value = Chain;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.inner.parse_ref(cmd, arg, value)?.parse::<Chain>().map_err(|_| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                "chain argument did not match any possible chain variant",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_clap_chain() {
        let args: ClapChain = ClapChain::parse_from(["foundry-cli", "--chain", "mainnet"]);
        assert_eq!(args.inner, Chain::Mainnet);
    }
}
