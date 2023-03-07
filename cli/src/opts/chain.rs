use clap::builder::{PossibleValuesParser, TypedValueParser};
use ethers::types::Chain as NamedChain;
use foundry_config::Chain;
use std::ffi::OsStr;
use strum::VariantNames;

/// Custom Clap value parser for [`Chain`]s.
///
/// Displays all possible chains when an invalid chain is provided.
#[derive(Clone, Debug)]
pub struct ChainValueParser {
    pub inner: PossibleValuesParser,
}

impl Default for ChainValueParser {
    fn default() -> Self {
        Self { inner: PossibleValuesParser::from(NamedChain::VARIANTS) }
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
        if let Some(id) = value.to_str().and_then(|value| value.parse::<u64>().ok()) {
            Ok(Chain::Id(id))
        } else {
            let string = self.inner.parse_ref(cmd, arg, value)?;
            let named = string.parse::<NamedChain>().expect("Already validated");
            Ok(Chain::Named(named))
        }
    }
}
