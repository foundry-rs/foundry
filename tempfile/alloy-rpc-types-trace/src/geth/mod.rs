//! Geth tracing types.

use crate::geth::{
    call::FlatCallFrame,
    mux::{MuxConfig, MuxFrame},
};
use alloy_primitives::{Bytes, B256, U256};
use alloy_rpc_types_eth::{state::StateOverride, BlockOverrides};
use serde::{de::DeserializeOwned, ser::SerializeMap, Deserialize, Serialize, Serializer};
use std::{collections::BTreeMap, time::Duration};
// re-exports
pub use self::{
    call::{CallConfig, CallFrame, CallLogFrame, FlatCallConfig},
    four_byte::FourByteFrame,
    noop::NoopFrame,
    pre_state::{
        AccountChangeKind, AccountState, DiffMode, DiffStateKind, PreStateConfig, PreStateFrame,
        PreStateMode,
    },
};

pub mod call;
pub mod four_byte;
pub mod mux;
pub mod noop;
pub mod pre_state;

/// Error when the inner tracer from [GethTrace] is mismatching to the target tracer.
#[derive(Debug, thiserror::Error)]
#[error("unexpected tracer")]
pub struct UnexpectedTracerError(pub GethTrace);

/// Result type for geth style transaction trace
pub type TraceResult = crate::common::TraceResult<GethTrace, String>;

/// blockTraceResult represents the results of tracing a single block when an entire chain is being
/// traced.
///
/// Ref <https://github.com/ethereum/go-ethereum/blob/ee530c0d5aa70d2c00ab5691a89ab431b73f8165/eth/tracers/api.go#L218-L222>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockTraceResult {
    /// Block number corresponding to the trace task
    pub block: U256,
    /// Block hash corresponding to the trace task
    pub hash: B256,
    /// Trace results produced by the trace task
    pub traces: Vec<TraceResult>,
}

/// Geth Default struct log trace frame
///
/// <https://github.com/ethereum/go-ethereum/blob/a9ef135e2dd53682d106c6a2aede9187026cc1de/eth/tracers/logger/logger.go#L406-L411>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultFrame {
    /// Whether the transaction failed
    pub failed: bool,
    /// How much gas was used.
    pub gas: u64,
    /// Output of the transaction
    #[serde(serialize_with = "alloy_serde::serialize_hex_string_no_prefix")]
    pub return_value: Bytes,
    /// Recorded traces of the transaction
    pub struct_logs: Vec<StructLog>,
}

/// Represents a struct log entry in a trace
///
/// <https://github.com/ethereum/go-ethereum/blob/366d2169fbc0e0f803b68c042b77b6b480836dbc/eth/tracers/logger/logger.go#L413-L426>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructLog {
    /// program counter
    pub pc: u64,
    /// opcode to be executed
    pub op: String,
    /// remaining gas
    pub gas: u64,
    /// cost for executing op
    #[serde(rename = "gasCost")]
    pub gas_cost: u64,
    /// Current call depth
    pub depth: u64,
    /// Error message if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// EVM stack
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<Vec<U256>>,
    /// Last call's return data. Enabled via enableReturnData
    #[serde(default, rename = "returnData", skip_serializing_if = "Option::is_none")]
    pub return_data: Option<Bytes>,
    /// ref <https://github.com/ethereum/go-ethereum/blob/366d2169fbc0e0f803b68c042b77b6b480836dbc/eth/tracers/logger/logger.go#L450-L452>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<Vec<String>>,
    /// Size of memory.
    #[serde(default, rename = "memSize", skip_serializing_if = "Option::is_none")]
    pub memory_size: Option<u64>,
    /// Storage slots of current contract read from and written to. Only emitted for SLOAD and
    /// SSTORE. Disabled via disableStorage
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_string_storage_map_opt"
    )]
    pub storage: Option<BTreeMap<B256, B256>>,
    /// Refund counter
    #[serde(default, rename = "refund", skip_serializing_if = "Option::is_none")]
    pub refund_counter: Option<u64>,
}

/// Tracing response objects
///
/// Note: This deserializes untagged, so it's possible that a custom javascript tracer response
/// matches another variant, for example a js tracer that returns `{}` would be deserialized as
/// [GethTrace::NoopTracer]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GethTrace {
    /// The response for the default struct log tracer
    Default(DefaultFrame),
    /// The response for call tracer
    CallTracer(CallFrame),
    /// The response for the flat call tracer
    FlatCallTracer(FlatCallFrame),
    /// The response for four byte tracer
    FourByteTracer(FourByteFrame),
    /// The response for pre-state byte tracer
    PreStateTracer(PreStateFrame),
    /// An empty json response
    NoopTracer(NoopFrame),
    /// The response for mux tracer
    MuxTracer(MuxFrame),
    /// Any other trace response, such as custom javascript response objects
    JS(serde_json::Value),
}

impl GethTrace {
    /// Try to convert the inner tracer to [DefaultFrame]
    pub fn try_into_default_frame(self) -> Result<DefaultFrame, UnexpectedTracerError> {
        match self {
            Self::Default(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [CallFrame]
    pub fn try_into_call_frame(self) -> Result<CallFrame, UnexpectedTracerError> {
        match self {
            Self::CallTracer(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [FlatCallFrame]
    pub fn try_into_flat_call_frame(self) -> Result<FlatCallFrame, UnexpectedTracerError> {
        match self {
            Self::FlatCallTracer(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [FourByteFrame]
    pub fn try_into_four_byte_frame(self) -> Result<FourByteFrame, UnexpectedTracerError> {
        match self {
            Self::FourByteTracer(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [PreStateFrame]
    pub fn try_into_pre_state_frame(self) -> Result<PreStateFrame, UnexpectedTracerError> {
        match self {
            Self::PreStateTracer(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [NoopFrame]
    pub fn try_into_noop_frame(self) -> Result<NoopFrame, UnexpectedTracerError> {
        match self {
            Self::NoopTracer(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [MuxFrame]
    pub fn try_into_mux_frame(self) -> Result<MuxFrame, UnexpectedTracerError> {
        match self {
            Self::MuxTracer(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }

    /// Try to convert the inner tracer to [serde_json::Value]
    pub fn try_into_json_value(self) -> Result<serde_json::Value, UnexpectedTracerError> {
        match self {
            Self::JS(inner) => Ok(inner),
            _ => Err(UnexpectedTracerError(self)),
        }
    }
}

impl Default for GethTrace {
    fn default() -> Self {
        Self::Default(DefaultFrame::default())
    }
}

impl From<DefaultFrame> for GethTrace {
    fn from(value: DefaultFrame) -> Self {
        Self::Default(value)
    }
}

impl From<FourByteFrame> for GethTrace {
    fn from(value: FourByteFrame) -> Self {
        Self::FourByteTracer(value)
    }
}

impl From<CallFrame> for GethTrace {
    fn from(value: CallFrame) -> Self {
        Self::CallTracer(value)
    }
}

impl From<FlatCallFrame> for GethTrace {
    fn from(value: FlatCallFrame) -> Self {
        Self::FlatCallTracer(value)
    }
}

impl From<PreStateFrame> for GethTrace {
    fn from(value: PreStateFrame) -> Self {
        Self::PreStateTracer(value)
    }
}

impl From<NoopFrame> for GethTrace {
    fn from(value: NoopFrame) -> Self {
        Self::NoopTracer(value)
    }
}

impl From<MuxFrame> for GethTrace {
    fn from(value: MuxFrame) -> Self {
        Self::MuxTracer(value)
    }
}

/// Available built-in tracers
///
/// See <https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers>
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GethDebugBuiltInTracerType {
    /// The 4byteTracer collects the function selectors of every function executed in the lifetime
    /// of a transaction, along with the size of the supplied call data. The result is a
    /// [FourByteFrame] where the keys are SELECTOR-CALLDATASIZE and the values are number of
    /// occurrences of this key.
    #[serde(rename = "4byteTracer")]
    FourByteTracer,
    /// The callTracer tracks all the call frames executed during a transaction, including depth 0.
    /// The result will be a nested list of call frames, resembling how EVM works. They form a tree
    /// with the top-level call at root and sub-calls as children of the higher levels.
    #[serde(rename = "callTracer")]
    CallTracer,
    /// Tracks all call frames of a transaction and returns them in a flat format, i.e. as opposed
    /// to the nested format of `callTracer`.
    #[serde(rename = "flatCallTracer")]
    FlatCallTracer,
    /// The prestate tracer operates in two distinct modes: prestate and diff.
    /// - In prestate mode, it retrieves the accounts required for executing a specified
    ///   transaction.
    /// - In diff mode, it identifies the changes between the transaction's initial and final
    ///   states, detailing the modifications caused by the transaction.
    ///
    /// By default, the prestateTracer is set to prestate mode. It reexecutes the given transaction
    /// and tracks every part of state that is accessed.
    ///
    /// This functionality is akin to a stateless witness, with the key distinction that this
    /// tracer does not provide any cryptographic proofs; it only returns the trie leaves.
    /// The output is an object where the keys correspond to account addresses.
    #[serde(rename = "prestateTracer")]
    PreStateTracer,
    /// This tracer is noop. It returns an empty object and is only meant for testing the setup.
    #[serde(rename = "noopTracer")]
    NoopTracer,
    /// The mux tracer is a tracer that can run multiple tracers at once.
    #[serde(rename = "muxTracer")]
    MuxTracer,
}

/// Available tracers
///
/// See <https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers> and <https://geth.ethereum.org/docs/developers/evm-tracing/custom-tracer>
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GethDebugTracerType {
    /// built-in tracer
    BuiltInTracer(GethDebugBuiltInTracerType),
    /// custom JS tracer
    JsTracer(String),
}

impl From<GethDebugBuiltInTracerType> for GethDebugTracerType {
    fn from(value: GethDebugBuiltInTracerType) -> Self {
        Self::BuiltInTracer(value)
    }
}

/// Configuration of the tracer
///
/// This is a simple wrapper around serde_json::Value.
/// with helpers for deserializing tracer configs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GethDebugTracerConfig(pub serde_json::Value);

// === impl GethDebugTracerConfig ===

impl GethDebugTracerConfig {
    /// Returns if this is a null object
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    /// Consumes the config and tries to deserialize it into the given type.
    pub fn from_value<T: DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.0)
    }

    /// Returns the [CallConfig] if it is a call config.
    pub fn into_call_config(self) -> Result<CallConfig, serde_json::Error> {
        if self.0.is_null() {
            return Ok(Default::default());
        }
        self.from_value()
    }

    /// Returns the [FlatCallConfig] if it is a call config.
    pub fn into_flat_call_config(self) -> Result<FlatCallConfig, serde_json::Error> {
        if self.0.is_null() {
            return Ok(Default::default());
        }
        self.from_value()
    }

    /// Returns the raw json value
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Returns the [PreStateConfig] if it is a prestate config.
    pub fn into_pre_state_config(self) -> Result<PreStateConfig, serde_json::Error> {
        if self.0.is_null() {
            return Ok(Default::default());
        }
        self.from_value()
    }

    /// Returns the [MuxConfig] if it is a mux config.
    pub fn into_mux_config(self) -> Result<MuxConfig, serde_json::Error> {
        if self.0.is_null() {
            return Ok(Default::default());
        }
        self.from_value()
    }
}

impl From<serde_json::Value> for GethDebugTracerConfig {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl From<CallConfig> for GethDebugTracerConfig {
    fn from(value: CallConfig) -> Self {
        Self(serde_json::to_value(value).expect("is serializable"))
    }
}
impl From<FlatCallConfig> for GethDebugTracerConfig {
    fn from(value: FlatCallConfig) -> Self {
        Self(serde_json::to_value(value).expect("is serializable"))
    }
}

impl From<PreStateConfig> for GethDebugTracerConfig {
    fn from(value: PreStateConfig) -> Self {
        Self(serde_json::to_value(value).expect("is serializable"))
    }
}

impl From<MuxConfig> for GethDebugTracerConfig {
    fn from(value: MuxConfig) -> Self {
        Self(serde_json::to_value(value).expect("is serializable"))
    }
}

/// Bindings for additional `debug_traceTransaction` options
///
/// See <https://geth.ethereum.org/docs/rpc/ns-debug#debug_tracetransaction>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GethDebugTracingOptions {
    /// The common tracing options
    #[serde(default, flatten)]
    pub config: GethDefaultTracingOptions,
    /// The custom tracer to use.
    ///
    /// If `None` then the default structlog tracer is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracer: Option<GethDebugTracerType>,
    /// Config specific to given `tracer`.
    ///
    /// Note default struct logger config are historically embedded in main object.
    ///
    /// tracerConfig is slated for Geth v1.11.0
    /// See <https://github.com/ethereum/go-ethereum/issues/26513>
    ///
    /// This could be [CallConfig] or [PreStateConfig] depending on the tracer.
    #[serde(default, skip_serializing_if = "GethDebugTracerConfig::is_null")]
    pub tracer_config: GethDebugTracerConfig,
    /// A string of decimal integers that overrides the JavaScript-based tracing calls default
    /// timeout of 5 seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
}

impl GethDebugTracingOptions {
    /// Creates a new instance with given [`GethDebugTracerType`] configured
    pub fn new_tracer(tracer: impl Into<GethDebugTracerType>) -> Self {
        Self::default().with_tracer(tracer.into())
    }

    /// Sets the tracer to use
    pub fn with_tracer(mut self, tracer: GethDebugTracerType) -> Self {
        self.tracer = Some(tracer);
        self
    }

    /// Creates new Options for [`GethDebugBuiltInTracerType::CallTracer`].
    pub fn call_tracer(config: CallConfig) -> Self {
        Self::new_tracer(GethDebugBuiltInTracerType::CallTracer).with_call_config(config)
    }

    /// Creates new Options for [`GethDebugBuiltInTracerType::FlatCallTracer`].
    pub fn flat_call_tracer(config: FlatCallConfig) -> Self {
        Self::new_tracer(GethDebugBuiltInTracerType::FlatCallTracer).with_config(config)
    }

    /// Creates new Options for [`GethDebugBuiltInTracerType::MuxTracer`].
    pub fn mux_tracer(config: MuxConfig) -> Self {
        Self::new_tracer(GethDebugBuiltInTracerType::MuxTracer).with_config(config)
    }

    /// Creates new options for [`GethDebugBuiltInTracerType::PreStateTracer`]
    pub fn prestate_tracer(config: PreStateConfig) -> Self {
        Self::new_tracer(GethDebugBuiltInTracerType::PreStateTracer).with_prestate_config(config)
    }

    /// Creates new options for [`GethDebugBuiltInTracerType::FourByteTracer`]
    pub fn four_byte_tracer() -> Self {
        Self::new_tracer(GethDebugBuiltInTracerType::FourByteTracer)
    }

    /// Creates an [`GethDebugTracerType::JsTracer`] with the given js code.
    pub fn js_tracer(code: impl Into<String>) -> Self {
        Self::new_tracer(GethDebugTracerType::JsTracer(code.into()))
    }

    /// Sets the timeout to use for tracing
    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(format!("{}ms", duration.as_millis()));
        self
    }

    /// Configures a [CallConfig]
    pub fn with_call_config(mut self, config: CallConfig) -> Self {
        self.tracer_config = config.into();
        self
    }

    /// Configures a [PreStateConfig]
    pub fn with_prestate_config(mut self, config: PreStateConfig) -> Self {
        self.tracer_config = config.into();
        self
    }

    /// Sets the tracer config
    pub fn with_config<T>(mut self, config: T) -> Self
    where
        T: Into<GethDebugTracerConfig>,
    {
        self.tracer_config = config.into();
        self
    }
}

/// Default tracing options for the struct logger.
///
/// These are all known general purpose tracer options that may or not be supported by a given
/// tracer. For example, the `enableReturnData` option is a noop on regular
/// `debug_trace{Transaction,Block}` calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GethDefaultTracingOptions {
    /// enable memory capture
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_memory: Option<bool>,
    /// Disable memory capture
    ///
    /// This is the opposite of `enable_memory`.
    ///
    /// Note: memory capture used to be enabled by default on geth, but has since been flipped <https://github.com/ethereum/go-ethereum/pull/23558> and is now disabled by default.
    /// However, at the time of writing this, Erigon still defaults to enabled and supports the
    /// `disableMemory` option. So we keep this option for compatibility, but if it's missing
    /// OR `enableMemory` is present `enableMemory` takes precedence.
    ///
    /// See also <https://github.com/paradigmxyz/reth/issues/3033>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_memory: Option<bool>,
    /// disable stack capture
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_stack: Option<bool>,
    /// Disable storage capture
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_storage: Option<bool>,
    /// Enable return data capture
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_return_data: Option<bool>,
    /// Disable return data capture
    ///
    /// This is the opposite of `enable_return_data`, and only supported for compatibility reasons.
    /// See also `disable_memory`.
    ///
    /// If `enable_return_data` is present, `enable_return_data` always takes precedence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_return_data: Option<bool>,
    /// print output during capture end
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<bool>,
    /// maximum length of output, but zero means unlimited
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

impl GethDefaultTracingOptions {
    /// Enables memory capture.
    pub const fn enable_memory(self) -> Self {
        self.with_enable_memory(true)
    }

    /// Disables memory capture.
    pub const fn disable_memory(self) -> Self {
        self.with_disable_memory(true)
    }

    /// Disables stack capture.
    pub const fn disable_stack(self) -> Self {
        self.with_disable_stack(true)
    }

    /// Disables storage capture.
    pub const fn disable_storage(self) -> Self {
        self.with_disable_storage(true)
    }

    /// Enables return data capture.
    pub const fn enable_return_data(self) -> Self {
        self.with_enable_return_data(true)
    }

    /// Disables return data capture.
    pub const fn disable_return_data(self) -> Self {
        self.with_disable_return_data(true)
    }

    /// Enables debug mode.
    pub const fn debug(self) -> Self {
        self.with_debug(true)
    }

    /// Sets the enable_memory field.
    pub const fn with_enable_memory(mut self, enable: bool) -> Self {
        self.enable_memory = Some(enable);
        self
    }

    /// Sets the disable_memory field.
    pub const fn with_disable_memory(mut self, disable: bool) -> Self {
        self.disable_memory = Some(disable);
        self
    }

    /// Sets the disable_stack field.
    pub const fn with_disable_stack(mut self, disable: bool) -> Self {
        self.disable_stack = Some(disable);
        self
    }

    /// Sets the disable_storage field.
    pub const fn with_disable_storage(mut self, disable: bool) -> Self {
        self.disable_storage = Some(disable);
        self
    }

    /// Sets the enable_return_data field.
    pub const fn with_enable_return_data(mut self, enable: bool) -> Self {
        self.enable_return_data = Some(enable);
        self
    }

    /// Sets the disable_return_data field.
    pub const fn with_disable_return_data(mut self, disable: bool) -> Self {
        self.disable_return_data = Some(disable);
        self
    }

    /// Sets the debug field.
    pub const fn with_debug(mut self, debug: bool) -> Self {
        self.debug = Some(debug);
        self
    }

    /// Sets the limit field.
    pub const fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Returns `true` if return data capture is enabled
    pub fn is_return_data_enabled(&self) -> bool {
        self.enable_return_data
            .or_else(|| self.disable_return_data.map(|disable| !disable))
            .unwrap_or(false)
    }

    /// Returns `true` if memory capture is enabled
    pub fn is_memory_enabled(&self) -> bool {
        self.enable_memory.or_else(|| self.disable_memory.map(|disable| !disable)).unwrap_or(false)
    }

    /// Returns `true` if stack capture is enabled
    pub fn is_stack_enabled(&self) -> bool {
        !self.disable_stack.unwrap_or(false)
    }

    /// Returns `true` if storage capture is enabled
    pub fn is_storage_enabled(&self) -> bool {
        !self.disable_storage.unwrap_or(false)
    }
}
/// Bindings for additional `debug_traceCall` options
///
/// See <https://geth.ethereum.org/docs/rpc/ns-debug#debug_tracecall>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GethDebugTracingCallOptions {
    /// All the options
    #[serde(flatten)]
    pub tracing_options: GethDebugTracingOptions,
    /// The state overrides to apply
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_overrides: Option<StateOverride>,
    /// The block overrides to apply
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_overrides: Option<BlockOverrides>,
}

impl GethDebugTracingCallOptions {
    /// Creates a new instance with the given tracing options
    pub const fn new(tracing_options: GethDebugTracingOptions) -> Self {
        Self { tracing_options, state_overrides: None, block_overrides: None }
    }

    /// Enables state overrides
    pub fn with_state_overrides(mut self, overrides: StateOverride) -> Self {
        self.state_overrides = Some(overrides);
        self
    }

    /// Enables block overrides
    pub fn with_block_overrides(mut self, overrides: BlockOverrides) -> Self {
        self.block_overrides = Some(overrides);
        self
    }

    /// Sets the tracing options
    pub fn with_tracing_options(mut self, options: GethDebugTracingOptions) -> Self {
        self.tracing_options = options;
        self
    }
}

impl From<GethDebugTracingOptions> for GethDebugTracingCallOptions {
    fn from(value: GethDebugTracingOptions) -> Self {
        Self::new(value)
    }
}

/// Serializes a storage map as a list of key-value pairs _without_ 0x-prefix
fn serialize_string_storage_map_opt<S: Serializer>(
    storage: &Option<BTreeMap<B256, B256>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match storage {
        None => s.serialize_none(),
        Some(storage) => {
            let mut m = s.serialize_map(Some(storage.len()))?;
            for (key, val) in storage {
                let key = format!("{:?}", key);
                let val = format!("{:?}", val);
                // skip the 0x prefix
                m.serialize_entry(&key.as_str()[2..], &val.as_str()[2..])?;
            }
            m.end()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    fn test_tracer_config() {
        let s = "{\"tracer\": \"callTracer\"}";
        let opts = serde_json::from_str::<GethDebugTracingOptions>(s).unwrap();
        assert_eq!(
            opts.tracer,
            Some(GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::CallTracer))
        );
        let _call_config = opts.tracer_config.clone().into_call_config().unwrap();
        let _prestate_config = opts.tracer_config.into_pre_state_config().unwrap();
    }

    #[test]
    fn test_memory_capture() {
        let mut config = GethDefaultTracingOptions::default();

        // by default false
        assert!(!config.is_memory_enabled());

        config.disable_memory = Some(false);
        // disable == false -> enable
        assert!(config.is_memory_enabled());

        config.enable_memory = Some(false);
        // enable == false -> disable
        assert!(!config.is_memory_enabled());
    }

    #[test]
    fn test_return_data_capture() {
        let mut config = GethDefaultTracingOptions::default();

        // by default false
        assert!(!config.is_return_data_enabled());

        config.disable_return_data = Some(false);
        // disable == false -> enable
        assert!(config.is_return_data_enabled());

        config.enable_return_data = Some(false);
        // enable == false -> disable
        assert!(!config.is_return_data_enabled());
    }

    // <https://etherscan.io/tx/0xd01212e8ab48d2fd2ea9c4f33f8670fd1cf0cfb09d2e3c6ceddfaf54152386e5>
    #[test]
    fn serde_default_frame() {
        let input = include_str!("../../test_data/default/structlogs_01.json");
        let _frame: DefaultFrame = serde_json::from_str(input).unwrap();
    }

    #[test]
    fn test_serialize_storage_map() {
        let s = r#"{"pc":3349,"op":"SLOAD","gas":23959,"gasCost":2100,"depth":1,"stack":[],"memory":[],"storage":{"6693dabf5ec7ab1a0d1c5bc58451f85d5e44d504c9ffeb75799bfdb61aa2997a":"0000000000000000000000000000000000000000000000000000000000000000"}}"#;
        let log: StructLog = serde_json::from_str(s).unwrap();
        let val = serde_json::to_value(&log).unwrap();
        let input = serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert_eq!(input, val);
    }

    #[test]
    fn test_trace_result_serde() {
        let s = r#"        {
            "result": {
                "from": "0xccc5499e15fedaaeaba68aeb79b95b20f725bc56",
                "gas": "0x186a0",
                "gasUsed": "0xdb91",
                "to": "0xdac17f958d2ee523a2206206994597c13d831ec7",
                "input": "0xa9059cbb000000000000000000000000e3f85a274c1edbea2f2498cf5978f41961cf8b5b0000000000000000000000000000000000000000000000000000000068c8f380",
                "value": "0x0",
                "type": "CALL"
            },
            "txHash": "0x7cc741c553d4098f319c894d9db208999ca49ee1b5c53f6a9992e687cbffb69e"
        }"#;
        let result: TraceResult = serde_json::from_str(s).unwrap();
        let hash = result.tx_hash().unwrap();
        assert_eq!(
            hash,
            "0x7cc741c553d4098f319c894d9db208999ca49ee1b5c53f6a9992e687cbffb69e"
                .parse::<B256>()
                .unwrap()
        );

        let de = serde_json::to_value(&result).unwrap();
        let val = serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert_eq!(val, de);
    }

    #[test]
    fn test_geth_trace_into_tracer() {
        let geth_trace = GethTrace::Default(DefaultFrame::default());
        let inner = geth_trace.try_into_default_frame();
        assert!(inner.is_ok());

        let geth_trace = GethTrace::CallTracer(CallFrame::default());
        let inner = geth_trace.try_into_call_frame();
        assert!(inner.is_ok());

        let geth_trace = GethTrace::FourByteTracer(FourByteFrame::default());
        let inner = geth_trace.try_into_four_byte_frame();
        assert!(inner.is_ok());

        let geth_trace = GethTrace::PreStateTracer(PreStateFrame::Default(PreStateMode::default()));
        let inner = geth_trace.try_into_pre_state_frame();
        assert!(inner.is_ok());

        let geth_trace = GethTrace::NoopTracer(NoopFrame::default());
        let inner = geth_trace.try_into_noop_frame();
        assert!(inner.is_ok());

        let geth_trace = GethTrace::MuxTracer(MuxFrame::default());
        let inner = geth_trace.try_into_mux_frame();
        assert!(inner.is_ok());

        let geth_trace = GethTrace::JS(serde_json::Value::Null);
        let inner = geth_trace.try_into_json_value();
        assert!(inner.is_ok());
    }

    #[test]
    fn test_geth_trace_into_tracer_wrong_tracer() {
        let geth_trace = GethTrace::Default(DefaultFrame::default());
        let inner = geth_trace.try_into_call_frame();
        assert!(inner.is_err());
        assert!(matches!(inner, Err(UnexpectedTracerError(_))));
    }
}
