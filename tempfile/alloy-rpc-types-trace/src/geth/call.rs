//! Geth call tracer types.

use crate::parity::LocalizedTransactionTrace;
use alloy_primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};

/// The response object for `debug_traceTransaction` with `"tracer": "callTracer"`.
///
/// <https://github.com/ethereum/go-ethereum/blob/91cb6f863a965481e51d5d9c0e5ccd54796fd967/eth/tracers/native/call.go#L44>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallFrame {
    /// The address of that initiated the call.
    pub from: Address,
    /// How much gas was left before the call.
    #[serde(default)]
    pub gas: U256,
    /// How much gas was used by the call.
    #[serde(default, rename = "gasUsed")]
    pub gas_used: U256,
    /// The address of the contract that was called.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    /// Calldata input.
    pub input: Bytes,
    /// Output of the call, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Bytes>,
    /// Error message, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why this call reverted, if it reverted.
    #[serde(default, rename = "revertReason", skip_serializing_if = "Option::is_none")]
    pub revert_reason: Option<String>,
    /// Recorded child calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<CallFrame>,
    /// Logs emitted by this call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<CallLogFrame>,
    /// Value transferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<U256>,
    /// The type of the call.
    #[serde(rename = "type")]
    pub typ: String,
}

/// Represents a recorded log that is emitted during a trace call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLogFrame {
    /// The address of the contract that was called.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    /// The topics of the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<B256>>,
    /// The data of the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Bytes>,
    /// The position of the log relative to subcalls within the same trace.
    #[serde(default, with = "alloy_serde::quantity::opt", skip_serializing_if = "Option::is_none")]
    pub position: Option<u64>,
}

/// The configuration for the call tracer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallConfig {
    /// When set to true, this will only trace the primary (top-level) call and not any sub-calls.
    /// It eliminates the additional processing for each call frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_top_call: Option<bool>,
    /// When set to true, this will include the logs emitted by the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with_log: Option<bool>,
}

impl CallConfig {
    /// Sets the only top call flag.
    pub const fn only_top_call(mut self) -> Self {
        self.only_top_call = Some(true);
        self
    }

    /// Sets the with log flag.
    pub const fn with_log(mut self) -> Self {
        self.with_log = Some(true);
        self
    }
}

/// The response object for `debug_traceTransaction` with `"tracer": "flatCallTracer"`.
///
/// That is equivalent to parity's [`LocalizedTransactionTrace`]
/// <https://github.com/ethereum/go-ethereum/blob/0dd173a727dd2d2409b8e401b22e85d20c25b71f/eth/tracers/native/call_flat.go#L62-L62>
pub type FlatCallFrame = Vec<LocalizedTransactionTrace>;

/// The configuration for the flat call tracer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlatCallConfig {
    /// If true, call tracer converts errors to parity format
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convert_parity_errors: Option<bool>,
    /// If true, call tracer includes calls to precompiled contracts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_precompiles: Option<bool>,
}

impl FlatCallConfig {
    /// Converts errors to parity format.
    pub const fn parity_errors(mut self) -> Self {
        self.convert_parity_errors = Some(true);
        self
    }

    /// Include calls to precompiled contracts.
    pub const fn with_precompiles(mut self) -> Self {
        self.include_precompiles = Some(true);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geth::*;
    use similar_asserts::assert_eq;

    // See <https://github.com/ethereum/go-ethereum/tree/master/eth/tracers/internal/tracetest/testdata>
    const DEFAULT: &str = include_str!("../../test_data/call_tracer/default.json");
    const LEGACY: &str = include_str!("../../test_data/call_tracer/legacy.json");
    const ONLY_TOP_CALL: &str = include_str!("../../test_data/call_tracer/only_top_call.json");
    const WITH_LOG: &str = include_str!("../../test_data/call_tracer/with_log.json");

    #[test]
    fn test_serialize_call_trace() {
        let mut opts = GethDebugTracingCallOptions::default();
        opts.tracing_options.config.disable_storage = Some(false);
        opts.tracing_options.tracer =
            Some(GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::CallTracer));
        opts.tracing_options.tracer_config =
            serde_json::to_value(CallConfig { only_top_call: Some(true), with_log: Some(true) })
                .unwrap()
                .into();

        assert_eq!(
            serde_json::to_string(&opts).unwrap(),
            r#"{"disableStorage":false,"tracer":"callTracer","tracerConfig":{"onlyTopCall":true,"withLog":true}}"#
        );
    }

    #[test]
    fn test_deserialize_call_trace() {
        let _trace: CallFrame = serde_json::from_str(DEFAULT).unwrap();
        let _trace: CallFrame = serde_json::from_str(LEGACY).unwrap();
        let _trace: CallFrame = serde_json::from_str(ONLY_TOP_CALL).unwrap();
        let _trace: CallFrame = serde_json::from_str(WITH_LOG).unwrap();
    }
}
