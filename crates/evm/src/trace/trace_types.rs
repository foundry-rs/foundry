//! Geth tracing types used for trace identification/decoding.
//! Temporal until we migrate to Alloy RPC types.

use std::{collections::BTreeMap, str::FromStr};

use alloy_primitives::{Address, Bytes, B256, U256};
use ethers::types::NameOrAddress;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// Parity tracing types

// `LocalizedTrace` in Parity
/// Trace-Filtering API trace type
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct Trace {
    /// Action
    pub action: Action,
    /// Result
    pub result: Option<Res>,
    /// Trace address
    #[serde(rename = "traceAddress")]
    pub trace_address: Vec<usize>,
    /// Subtraces
    pub subtraces: usize,
    /// Transaction position
    #[serde(rename = "transactionPosition")]
    pub transaction_position: Option<usize>,
    /// Transaction hash
    #[serde(rename = "transactionHash")]
    pub transaction_hash: Option<B256>,
    /// Block Number
    #[serde(rename = "blockNumber")]
    pub block_number: u64,
    /// Block Hash
    #[serde(rename = "blockHash")]
    pub block_hash: B256,
    /// Action Type
    #[serde(rename = "type")]
    pub action_type: ActionType,
    /// Error, See also [`TraceError`]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Res {
    /// Call
    Call(CallResult),
    /// Create
    Create(CreateResult),
    /// None
    #[default]
    None,
}

/// Action
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "lowercase")]
pub enum Action {
    /// Call
    Call(Call),
    /// Create
    Create(Create),
    /// Suicide
    Suicide(Suicide),
    /// Reward
    Reward(Reward),
}

/// An external action type.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    /// Contract call.
    Call,
    /// Contract creation.
    Create,
    /// Contract suicide.
    Suicide,
    /// A block reward.
    Reward,
}

/// Call Result
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct CallResult {
    /// Gas used
    #[serde(rename = "gasUsed")]
    pub gas_used: U256,
    /// Output bytes
    pub output: Bytes,
}

/// Create Result
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct CreateResult {
    /// Gas used
    #[serde(rename = "gasUsed")]
    pub gas_used: U256,
    /// Code
    pub code: Bytes,
    /// Assigned address
    pub address: Address,
}

/// Call response
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Call {
    /// Sender
    pub from: Address,
    /// Recipient
    pub to: Address,
    /// Transferred Value
    pub value: U256,
    /// Gas
    pub gas: U256,
    /// Input data
    pub input: Bytes,
    /// The type of the call.
    #[serde(rename = "callType")]
    pub call_type: CallType,
}

/// Call type.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum CallType {
    /// None
    #[default]
    #[serde(rename = "none")]
    None,
    /// Call
    #[serde(rename = "call")]
    Call,
    /// Call code
    #[serde(rename = "callcode")]
    CallCode,
    /// Delegate call
    #[serde(rename = "delegatecall")]
    DelegateCall,
    /// Static call
    #[serde(rename = "staticcall")]
    StaticCall,
}

/// Create response
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Create {
    /// Sender
    pub from: Address,
    /// Value
    pub value: U256,
    /// Gas
    pub gas: U256,
    /// Initialization code
    pub init: Bytes,
}

/// Suicide
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Suicide {
    /// Address.
    pub address: Address,
    /// Refund address.
    #[serde(rename = "refundAddress")]
    pub refund_address: Address,
    /// Balance.
    pub balance: U256,
}

/// Reward action
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Reward {
    /// Author's address.
    pub author: Address,
    /// Reward amount.
    pub value: U256,
    /// Reward type.
    #[serde(rename = "rewardType")]
    pub reward_type: RewardType,
}

/// Reward type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum RewardType {
    /// Block
    #[serde(rename = "block")]
    Block,
    /// Uncle
    #[serde(rename = "uncle")]
    Uncle,
    /// EmptyStep (AuthorityRound)
    #[serde(rename = "emptyStep")]
    EmptyStep,
    /// External (attributed as part of an external protocol)
    #[serde(rename = "external")]
    External,
}

// Geth tracing types

// https://github.com/ethereum/go-ethereum/blob/91cb6f863a965481e51d5d9c0e5ccd54796fd967/eth/tracers/native/call.go#L44
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallFrame {
    #[serde(rename = "type")]
    pub typ: String,
    pub from: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<NameOrAddress>,
    #[serde(
        default,
        deserialize_with = "deserialize_stringified_numeric_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<U256>,
    #[serde(default, deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(default, deserialize_with = "deserialize_stringified_numeric", rename = "gasUsed")]
    pub gas_used: U256,
    pub input: Bytes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Bytes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls: Option<Vec<CallFrame>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs: Option<Vec<CallLogFrame>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLogFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<B256>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Bytes>,
}

// https://github.com/ethereum/go-ethereum/blob/91cb6f863a965481e51d5d9c0e5ccd54796fd967/eth/tracers/native/prestate.go#L38
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PreStateFrame {
    Default(PreStateMode),
    Diff(DiffMode),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreStateMode(pub BTreeMap<Address, AccountState>);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffMode {
    pub pre: BTreeMap<Address, AccountState>,
    pub post: BTreeMap<Address, AccountState>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountState {
    #[serde(
        default,
        deserialize_with = "deserialize_stringified_numeric_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub balance: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_stringified_numeric_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub nonce: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<BTreeMap<B256, B256>>,
}

// https://github.com/ethereum/go-ethereum/blob/91cb6f863a965481e51d5d9c0e5ccd54796fd967/eth/tracers/native/4byte.go#L48
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FourByteFrame(pub BTreeMap<String, u64>);

// https://github.com/ethereum/go-ethereum/blob/91cb6f863a965481e51d5d9c0e5ccd54796fd967/eth/tracers/native/noop.go#L34
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoopFrame(BTreeMap<Null, Null>);
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
struct Null;

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GethTraceFrame {
    Default(DefaultFrame),
    NoopTracer(NoopFrame),
    FourByteTracer(FourByteFrame),
    CallTracer(CallFrame),
    PreStateTracer(PreStateFrame),
}

impl From<DefaultFrame> for GethTraceFrame {
    fn from(value: DefaultFrame) -> Self {
        GethTraceFrame::Default(value)
    }
}

impl From<FourByteFrame> for GethTraceFrame {
    fn from(value: FourByteFrame) -> Self {
        GethTraceFrame::FourByteTracer(value)
    }
}

impl From<CallFrame> for GethTraceFrame {
    fn from(value: CallFrame) -> Self {
        GethTraceFrame::CallTracer(value)
    }
}

impl From<PreStateFrame> for GethTraceFrame {
    fn from(value: PreStateFrame) -> Self {
        GethTraceFrame::PreStateTracer(value)
    }
}

impl From<NoopFrame> for GethTraceFrame {
    fn from(value: NoopFrame) -> Self {
        GethTraceFrame::NoopTracer(value)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum GethTraceResult {
    ResultKnown { result: GethTraceFrame },
    ResultUnknown { result: Value },
    DefaultKnown(GethTraceFrame),
    DefaultUnknown(Value),
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(from = "GethTraceResult")]
#[serde(untagged)]
pub enum GethTrace {
    Known(GethTraceFrame),
    Unknown(Value),
}

impl From<GethTraceResult> for GethTrace {
    fn from(value: GethTraceResult) -> Self {
        match value {
            GethTraceResult::DefaultKnown(t) => GethTrace::Known(t),
            GethTraceResult::DefaultUnknown(v) => GethTrace::Unknown(v),
            GethTraceResult::ResultKnown { result } => GethTrace::Known(result),
            GethTraceResult::ResultUnknown { result } => GethTrace::Unknown(result),
        }
    }
}

impl From<GethTraceFrame> for GethTrace {
    fn from(value: GethTraceFrame) -> Self {
        GethTrace::Known(value)
    }
}

impl From<Value> for GethTrace {
    fn from(value: Value) -> Self {
        GethTrace::Unknown(value)
    }
}

// https://github.com/ethereum/go-ethereum/blob/a9ef135e2dd53682d106c6a2aede9187026cc1de/eth/tracers/logger/logger.go#L406-L411
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultFrame {
    pub failed: bool,
    #[serde(deserialize_with = "deserialize_stringified_numeric")]
    pub gas: U256,
    #[serde(rename = "returnValue")]
    pub return_value: Bytes,
    #[serde(rename = "structLogs")]
    pub struct_logs: Vec<StructLog>,
}

// https://github.com/ethereum/go-ethereum/blob/366d2169fbc0e0f803b68c042b77b6b480836dbc/eth/tracers/logger/logger.go#L413-L426
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructLog {
    pub depth: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub gas: u64,
    #[serde(rename = "gasCost")]
    pub gas_cost: u64,
    /// ref <https://github.com/ethereum/go-ethereum/blob/366d2169fbc0e0f803b68c042b77b6b480836dbc/eth/tracers/logger/logger.go#L450-L452>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<Vec<String>>,
    pub op: String,
    pub pc: u64,
    #[serde(default, rename = "refund", skip_serializing_if = "Option::is_none")]
    pub refund_counter: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<Vec<U256>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<BTreeMap<B256, B256>>,
}

/// Bindings for additional `debug_traceTransaction` options
///
/// See <https://geth.ethereum.org/docs/rpc/ns-debug#debug_tracetransaction>
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GethDebugTracingOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_storage: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_stack: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_memory: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_return_data: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracer: Option<GethDebugTracerType>,
    /// tracerConfig is slated for Geth v1.11.0
    /// See <https://github.com/ethereum/go-ethereum/issues/26513>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracer_config: Option<GethDebugTracerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
}

/// Available built-in tracers
///
/// See <https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers>
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub enum GethDebugBuiltInTracerType {
    #[serde(rename = "4byteTracer")]
    FourByteTracer,
    #[serde(rename = "callTracer")]
    CallTracer,
    #[serde(rename = "prestateTracer")]
    PreStateTracer,
    #[serde(rename = "noopTracer")]
    NoopTracer,
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GethDebugBuiltInTracerConfig {
    CallTracer(CallConfig),
    PreStateTracer(PreStateConfig),
}

/// Available tracers
///
/// See <https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers> and <https://geth.ethereum.org/docs/developers/evm-tracing/custom-tracer>
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GethDebugTracerType {
    /// built-in tracer
    BuiltInTracer(GethDebugBuiltInTracerType),

    /// custom JS tracer
    JsTracer(String),
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GethDebugTracerConfig {
    /// built-in tracer
    BuiltInTracer(GethDebugBuiltInTracerConfig),

    /// custom JS tracer
    JsTracer(serde_json::Value),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_top_call: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with_log: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreStateConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_mode: Option<bool>,
}

// Deserializing utilities

pub fn deserialize_stringified_numeric<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let num = StringifiedNumeric::deserialize(deserializer)?;
    num.try_into().map_err(serde::de::Error::custom)
}

/// Helper type to parse numeric strings, `u64` and `U256`
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StringifiedNumeric {
    String(String),
    U256(U256),
    Num(serde_json::Number),
}

impl TryFrom<StringifiedNumeric> for U256 {
    type Error = String;

    fn try_from(value: StringifiedNumeric) -> Result<Self, Self::Error> {
        match value {
            StringifiedNumeric::U256(n) => Ok(n),
            StringifiedNumeric::Num(n) => {
                Ok(U256::from_str(&n.to_string()).map_err(|err| err.to_string())?)
            }
            StringifiedNumeric::String(s) => {
                if let Ok(val) = s.parse::<u128>() {
                    Ok(U256::from(val))
                } else if s.starts_with("0x") {
                    U256::from_str_radix(&s, 16).map_err(|err| err.to_string())
                } else {
                    U256::from_str(&s).map_err(|err| err.to_string())
                }
            }
        }
    }
}

/// Supports parsing numbers as strings
///
/// See <https://github.com/gakonst/ethers-rs/issues/1507>
pub fn deserialize_stringified_numeric_opt<'de, D>(
    deserializer: D,
) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    if let Some(num) = Option::<StringifiedNumeric>::deserialize(deserializer)? {
        num.try_into().map(Some).map_err(serde::de::Error::custom)
    } else {
        Ok(None)
    }
}
