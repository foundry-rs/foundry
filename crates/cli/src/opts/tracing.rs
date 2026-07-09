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
    /// Returns whether labels should be disabled.
    pub const fn disable_labels(&self, config: &TracingConfig) -> bool {
        self.disable_labels || config.disable_labels
    }

    /// Returns the configured trace depth.
    pub const fn trace_depth(&self, config: &TracingConfig) -> Option<usize> {
        if self.trace_depth.is_some() { self.trace_depth } else { config.trace_depth }
    }

    /// Returns whether internal trace decoding is enabled.
    pub const fn decode_internal(&self, config: &TracingConfig) -> bool {
        self.decode_internal || config.decode_internal
    }

    /// Returns the CLI labels as an address map.
    pub fn parsed_labels(&self) -> AddressHashMap<String> {
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
