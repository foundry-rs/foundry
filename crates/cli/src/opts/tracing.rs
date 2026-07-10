//! CLI arguments for configuring trace rendering.

use std::str::FromStr;

use alloy_primitives::{Address, map::AddressHashMap};
use clap::{Parser, ValueHint};
use foundry_config::TracingConfig;
use serde::Serialize;

/// CLI arguments for trace rendering.
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(about = None, long_about = None)]
pub struct TracingArgs {
    /// Identify internal functions in traces.
    ///
    /// This will trace internal functions and decode stack parameters.
    ///
    /// Parameters stored in memory (such as bytes or arrays) are currently decoded only when a
    /// single function is matched, similarly to `--debug`, for performance reasons.
    #[arg(long, help_heading = "Trace options")]
    #[serde(skip)]
    pub decode_internal: bool,

    /// Maximum depth of rendered traces.
    #[arg(long, value_name = "DEPTH", help_heading = "Trace options")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_depth: Option<usize>,

    /// Disable labels in traces.
    #[arg(long, help_heading = "Trace options")]
    #[serde(skip)]
    pub disable_labels: bool,

    /// Hide addresses in trace parameters when a label is available.
    #[arg(long, help_heading = "Trace options")]
    #[serde(skip)]
    pub compact_labels: bool,

    /// Label addresses in traces.
    ///
    /// Example: 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth
    #[arg(
        long = "labels",
        visible_alias = "label",
        value_name = "ADDRESS:LABEL",
        value_hint = ValueHint::Other,
        help_heading = "Trace options"
    )]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

impl TracingArgs {
    /// Resolves CLI overrides against the configured trace rendering settings.
    pub fn resolve(&self, config: &TracingConfig, verbosity: u8) -> TracingConfig {
        let mut tracing = config.clone();
        if verbosity > 0 {
            tracing.verbosity = verbosity;
        }
        tracing.labels.extend(self.parsed_labels());
        tracing.disable_labels |= self.disable_labels;
        tracing.compact_labels |= self.compact_labels;
        tracing.trace_depth = self.trace_depth.or(tracing.trace_depth);
        tracing.decode_internal |= self.decode_internal;
        tracing
    }

    fn parsed_labels(&self) -> AddressHashMap<String> {
        self.labels
            .iter()
            .filter_map(|label| {
                let (address, label) = label.split_once(':')?;
                let address = Address::from_str(address).ok()?;
                Some((address, label.to_string()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn resolve_merges_cli_overrides() {
        let address = address!("0x0000000000000000000000000000000000000001");
        let config = TracingConfig {
            verbosity: 2,
            labels: AddressHashMap::from_iter([(address, "config".to_string())]),
            disable_labels: false,
            compact_labels: false,
            trace_depth: Some(1),
            decode_internal: false,
        };
        let args = TracingArgs {
            decode_internal: true,
            trace_depth: Some(2),
            disable_labels: true,
            compact_labels: true,
            labels: vec![format!("{address}:cli")],
        };

        let tracing = args.resolve(&config, 3);
        assert_eq!(tracing.verbosity, 3);
        assert_eq!(tracing.labels.get(&address), Some(&"cli".to_string()));
        assert!(tracing.disable_labels);
        assert!(tracing.compact_labels);
        assert_eq!(tracing.trace_depth, Some(2));
        assert!(tracing.decode_internal);
    }

    #[test]
    fn explicit_verbosity_overrides_config() {
        let config = TracingConfig { verbosity: 5, ..Default::default() };

        assert_eq!(TracingArgs::default().resolve(&config, 1).verbosity, 1);
        assert_eq!(TracingArgs::default().resolve(&config, 0).verbosity, 5);
    }
}
