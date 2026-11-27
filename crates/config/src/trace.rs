//! Configuration for trace rendering.

use alloy_primitives::map::AddressHashMap;
use serde::{Deserialize, Serialize};

/// Configuration for trace rendering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TracingConfig {
    /// Minimum verbosity used when rendering traces.
    ///
    /// Rendering uses the greater of this value and the command's own verbosity (its `-v` flags,
    /// or the top-level `verbosity` setting), so command-line flags can raise but not lower it.
    /// This value only controls traces: console logs still follow the command's verbosity.
    ///
    /// In `forge test`:
    ///
    /// - `0`-`2`: no traces are displayed.
    /// - `3`: traces are displayed for failing tests.
    /// - `4`: traces are displayed for all tests, and setup traces for failing tests.
    /// - `5`: traces and setup traces are always displayed, including storage changes.
    ///
    /// `forge script`, `cast run` and `cast call` also render storage changes from level `5`.
    pub verbosity: u8,
    /// Labels to display for addresses in decoded traces.
    ///
    /// Each key is an address and each value is its label. Labels passed with `--label` or
    /// `--labels` extend this table and take precedence for duplicate addresses. These labels are
    /// also known to the `vm.getLabel` cheatcode.
    ///
    /// The legacy `[labels]` table and profile-level `labels` setting are deprecated; move them
    /// here.
    ///
    /// ```toml
    /// [profile.default.tracing.labels]
    /// "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045" = "vitalik.eth"
    /// ```
    #[serde(default, skip_serializing_if = "AddressHashMap::is_empty")]
    pub labels: AddressHashMap<String>,
    /// Whether to disable address labels in rendered traces.
    ///
    /// The `--disable-labels` flag also enables this; it cannot turn off a `true` value.
    pub disable_labels: bool,
    /// Whether to hide an address in trace parameters when a label is available.
    ///
    /// When enabled, a labeled address is displayed as its label alone instead of
    /// `label: [address]`. The `--compact-labels` flag also enables this.
    pub compact_labels: bool,
    /// Maximum depth of rendered traces.
    ///
    /// Calls nested deeper than this are left out of rendered traces and of trace JSON output.
    /// When unset, traces are rendered at every depth. The `--trace-depth` flag overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_depth: Option<usize>,
    /// Whether to identify internal functions in traces and decode their stack parameters.
    ///
    /// Dynamic values stored in memory, such as `bytes` or arrays, are decoded only when Foundry
    /// can match a single function, for performance reasons. In `forge test` that means exactly
    /// one matching test. The `--decode-internal` flag also enables this. It has no effect on
    /// traces fetched with an RPC call tracer.
    pub decode_internal: bool,
    /// Cumulative time budget, in seconds, for identifying trace contracts through Sourcify and
    /// Etherscan.
    ///
    /// Once the budget is spent, Foundry renders the trace with the identification results it
    /// has already received. The same value also bounds fetching verified sources for
    /// `decode_external_storage`.
    ///
    /// Set it to `0` to disable all external trace identification, including OpenChain
    /// signature lookups. The `--disable-external-identification` flag does the same for a
    /// single command. Neither prevents Foundry from downloading compiler versions.
    pub external_identification_timeout: u64,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            verbosity: 0,
            labels: Default::default(),
            disable_labels: false,
            compact_labels: false,
            trace_depth: None,
            decode_internal: false,
            external_identification_timeout: 5,
        }
    }
}
