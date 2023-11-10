use clap::builder::{PossibleValuesParser, TypedValueParser};
use eyre::Result;
use foundry_config::{Chain, NamedChain};
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
        let s =
            value.to_str().ok_or_else(|| clap::Error::new(clap::error::ErrorKind::InvalidUtf8))?;
        if let Ok(id) = s.parse() {
            Ok(Chain::Id(id))
        } else {
            // NamedChain::VARIANTS is a subset of all possible variants, since there are aliases:
            // mumbai instead of polygon-mumbai etc
            //
            // Parse first as NamedChain, if it fails parse with NamedChain::VARIANTS for displaying
            // the error to the user
            s.parse()
                .map_err(|_| self.inner.parse_ref(cmd, arg, value).unwrap_err())
                .map(Chain::Named)
        }
    }
}
