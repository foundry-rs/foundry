//! Types for trace module.
//!
//! See <https://openethereum.github.io/JSONRPC-trace-module>

use alloy_primitives::{Address, BlockHash, Bytes, TxHash, B256, U256, U64};
use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
};

/// Different Trace diagnostic targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceType {
    /// Default trace
    #[default]
    Trace,
    /// Provides a full trace of the VM’s state throughout the execution of the transaction,
    /// including for any subcalls.
    VmTrace,
    /// Provides information detailing all altered portions of the Ethereum state made due to the
    /// execution of the transaction.
    StateDiff,
}

/// The Outcome of a traced transaction with optional settings
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceResults {
    /// Output of the trace
    #[serde(deserialize_with = "alloy_serde::null_as_default")]
    pub output: Bytes,
    /// Enabled if [TraceType::StateDiff] is provided
    pub state_diff: Option<StateDiff>,
    /// Enabled if [TraceType::Trace] is provided, otherwise an empty vec
    #[serde(default)]
    pub trace: Vec<TransactionTrace>,
    /// Enabled if [TraceType::VmTrace] is provided
    pub vm_trace: Option<VmTrace>,
}

// === impl TraceResults ===

impl TraceResults {
    /// Sets the gas used of the root trace.
    ///
    /// The root trace's gasUsed should mirror the actual gas used by the transaction.
    ///
    /// This allows setting it manually by consuming the execution result's gas for example.
    pub fn set_root_trace_gas_used(&mut self, gas_used: u64) {
        if let Some(r) = self.trace.first_mut().and_then(|t| t.result.as_mut()) {
            r.set_gas_used(gas_used)
        }
    }
}

/// A `FullTrace` with an additional transaction hash
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceResultsWithTransactionHash {
    /// The recorded trace.
    #[serde(flatten)]
    pub full_trace: TraceResults,
    /// Hash of the traced transaction.
    #[doc(alias = "tx_hash")]
    pub transaction_hash: B256,
}

/// A changed value
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedType<T> {
    /// Original value
    pub from: T,
    /// New value
    pub to: T,
}

/// Represents how a value changed.
///
/// This is used for statediff.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delta<T> {
    /// Existing value didn't change.
    #[default]
    #[serde(rename = "=")]
    Unchanged,
    /// New storage value added.
    #[serde(rename = "+")]
    Added(T),
    /// Existing storage value removed.
    #[serde(rename = "-")]
    Removed(T),
    /// Existing storage value changed.
    #[serde(rename = "*")]
    Changed(ChangedType<T>),
}

// === impl Delta ===

impl<T> Delta<T> {
    /// Creates a new [Delta::Changed] variant
    pub const fn changed(from: T, to: T) -> Self {
        Self::Changed(ChangedType { from, to })
    }

    /// Returns true if the value is unchanged
    pub const fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }

    /// Returns true if the value is added
    pub const fn is_added(&self) -> bool {
        matches!(self, Self::Added(_))
    }

    /// Returns true if the value is removed
    pub const fn is_removed(&self) -> bool {
        matches!(self, Self::Removed(_))
    }

    /// Returns true if the value is changed
    pub const fn is_changed(&self) -> bool {
        matches!(self, Self::Changed(_))
    }
}

/// The diff of an account after a transaction
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiff {
    /// How the balance changed, if at all
    pub balance: Delta<U256>,
    /// How the code changed, if at all
    pub code: Delta<Bytes>,
    /// How the nonce changed, if at all
    // TODO: Change type to `u64` and write custom serde for `Delta`
    pub nonce: Delta<U64>,
    /// All touched/changed storage values
    pub storage: BTreeMap<B256, Delta<B256>>,
}

/// New-type for list of account diffs
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StateDiff(pub BTreeMap<Address, AccountDiff>);

impl Deref for StateDiff {
    type Target = BTreeMap<Address, AccountDiff>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StateDiff {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Represents the various types of actions recorded during tracing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "action")]
pub enum Action {
    /// Regular call
    Call(CallAction),
    /// A CREATE call
    Create(CreateAction),
    /// Parity style traces never renamed suicide to selfdestruct: <https://eips.ethereum.org/EIPS/eip-6>
    ///
    /// For compatibility reasons, this is serialized as `suicide`: <https://github.com/paradigmxyz/reth/issues/3721>
    #[serde(rename = "suicide", alias = "selfdestruct")]
    Selfdestruct(SelfdestructAction),
    /// Rewards if any (pre POS)
    Reward(RewardAction),
}

impl Default for Action {
    fn default() -> Self {
        Self::Call(CallAction::default())
    }
}

impl Action {
    /// Returns true if this is a call action
    pub const fn is_call(&self) -> bool {
        matches!(self, Self::Call(_))
    }

    /// Returns true if this is a create action
    pub const fn is_create(&self) -> bool {
        matches!(self, Self::Create(_))
    }

    /// Returns true if this is a selfdestruct action
    pub const fn is_selfdestruct(&self) -> bool {
        matches!(self, Self::Selfdestruct(_))
    }
    /// Returns true if this is a reward action
    pub const fn is_reward(&self) -> bool {
        matches!(self, Self::Reward(_))
    }

    /// Returns the [`CallAction`] if it is [`Action::Call`]
    pub const fn as_call(&self) -> Option<&CallAction> {
        match self {
            Self::Call(action) => Some(action),
            _ => None,
        }
    }

    /// Returns the [`CreateAction`] if it is [`Action::Create`]
    pub const fn as_create(&self) -> Option<&CreateAction> {
        match self {
            Self::Create(action) => Some(action),
            _ => None,
        }
    }

    /// Returns the [`SelfdestructAction`] if it is [`Action::Selfdestruct`]
    pub const fn as_selfdestruct(&self) -> Option<&SelfdestructAction> {
        match self {
            Self::Selfdestruct(action) => Some(action),
            _ => None,
        }
    }

    /// Returns the [`RewardAction`] if it is [`Action::Reward`]
    pub const fn as_reward(&self) -> Option<&RewardAction> {
        match self {
            Self::Reward(action) => Some(action),
            _ => None,
        }
    }

    /// Returns what kind of action this is
    pub const fn kind(&self) -> ActionType {
        match self {
            Self::Call(_) => ActionType::Call,
            Self::Create(_) => ActionType::Create,
            Self::Selfdestruct(_) => ActionType::Selfdestruct,
            Self::Reward(_) => ActionType::Reward,
        }
    }
}

/// An external action type.
///
/// Used as enum identifier for [Action]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    /// Contract call.
    Call,
    /// Contract creation.
    Create,
    /// Contract suicide/selfdestruct.
    #[serde(rename = "suicide", alias = "selfdestruct")]
    Selfdestruct,
    /// A block reward.
    Reward,
}

/// Call type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallType {
    /// None
    #[default]
    None,
    /// Call
    Call,
    /// Call code
    CallCode,
    /// Delegate call
    DelegateCall,
    /// Static call
    StaticCall,
    /// Authorized call
    AuthCall,
}

/// Represents a certain [CallType] of a _call_ or message transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CallAction {
    /// Address of the sending account.
    pub from: Address,
    /// The type of the call.
    pub call_type: CallType,
    /// The gas available for executing the call.
    #[serde(with = "alloy_serde::quantity")]
    pub gas: u64,
    /// The input data provided to the call.
    pub input: Bytes,
    /// Address of the destination/target account.
    pub to: Address,
    /// Value transferred to the destination account.
    pub value: U256,
}

/// Creation method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreationMethod {
    /// None
    #[default]
    None,
    /// Create
    Create,
    /// Create2
    Create2,
    /// EofCreate
    EofCreate,
}

/// Represents a _create_ action, either a `CREATE` operation or a CREATE transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CreateAction {
    /// The address of the creator.
    pub from: Address,
    /// The gas available for the creation init code.
    #[serde(with = "alloy_serde::quantity")]
    pub gas: u64,
    /// The init code.
    pub init: Bytes,
    /// The value with which the new account is endowed.
    pub value: U256,
    /// The contract creation method.
    // Note: this deserializes default because it's not yet supported by all clients
    #[serde(default)]
    pub creation_method: CreationMethod,
}

/// What kind of reward.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RewardType {
    /// Block rewards
    Block,
    /// Reward for uncle block
    Uncle,
}

/// Recorded reward of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardAction {
    /// Author's address.
    pub author: Address,
    /// Reward type.
    pub reward_type: RewardType,
    /// Reward amount.
    pub value: U256,
}

/// Represents a _selfdestruct_ action fka `suicide`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfdestructAction {
    /// destroyed/suicided address.
    pub address: Address,
    /// Balance of the contract just before it was destroyed.
    pub balance: U256,
    /// destroyed contract heir.
    pub refund_address: Address,
}

/// Outcome of a CALL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallOutput {
    /// Gas used by the call.
    #[serde(with = "alloy_serde::quantity")]
    pub gas_used: u64,
    /// The output data of the call.
    pub output: Bytes,
}

/// Outcome of a CREATE.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOutput {
    /// Address of the created contract.
    pub address: Address,
    /// Contract code.
    pub code: Bytes,
    /// Gas used by the call.
    #[serde(with = "alloy_serde::quantity")]
    pub gas_used: u64,
}

/// Represents the output of a trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TraceOutput {
    /// Output of a regular call transaction.
    Call(CallOutput),
    /// Output of a CREATE transaction.
    Create(CreateOutput),
}

impl TraceOutput {
    /// Returns the output of this trace.
    pub const fn output(&self) -> &Bytes {
        match self {
            Self::Call(call) => &call.output,
            Self::Create(create) => &create.code,
        }
    }

    /// Consumes the output of this trace.
    pub fn into_output(self) -> Bytes {
        match self {
            Self::Call(call) => call.output,
            Self::Create(create) => create.code,
        }
    }

    /// Returns the gas used by this trace.
    pub const fn gas_used(&self) -> u64 {
        match self {
            Self::Call(call) => call.gas_used,
            Self::Create(create) => create.gas_used,
        }
    }

    /// Sets the gas used by this trace.
    pub fn set_gas_used(&mut self, gas_used: u64) {
        match self {
            Self::Call(call) => call.gas_used = gas_used,
            Self::Create(create) => create.gas_used = gas_used,
        }
    }
}

/// A parity style trace of a transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[doc(alias = "TxTrace")]
pub struct TransactionTrace {
    /// Represents what kind of trace this is
    #[serde(flatten)]
    pub action: Action,
    /// The error message if the transaction failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Output of the trace, can be CALL or CREATE
    #[serde(default)]
    pub result: Option<TraceOutput>,
    /// How many subtraces this trace has.
    pub subtraces: usize,
    /// The identifier of this transaction trace in the set.
    ///
    /// This gives the exact location in the call trace
    /// [index in root CALL, index in first CALL, index in second CALL, …].
    pub trace_address: Vec<usize>,
}

/// A wrapper for [TransactionTrace] that includes additional information about the transaction.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[doc(alias = "LocalizedTxTrace")]
pub struct LocalizedTransactionTrace {
    /// Trace of the transaction and its result.
    #[serde(flatten)]
    pub trace: TransactionTrace,
    /// Hash of the block, if not pending.
    ///
    /// Note: this deviates from <https://openethereum.github.io/JSONRPC-trace-module#trace_transaction> which always returns a block number
    pub block_hash: Option<BlockHash>,
    /// Block number the transaction is included in, None if pending.
    ///
    /// Note: this deviates from <https://openethereum.github.io/JSONRPC-trace-module#trace_transaction> which always returns a block number
    pub block_number: Option<u64>,
    /// Hash of the transaction
    #[doc(alias = "tx_hash")]
    pub transaction_hash: Option<TxHash>,
    /// Transaction index within the block, None if pending.
    #[doc(alias = "tx_position", alias = "transaction_index", alias = "tx_index")]
    pub transaction_position: Option<u64>,
}

// Implement Serialize manually to ensure consistent ordering of fields to match other client's
// format
impl Serialize for LocalizedTransactionTrace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("LocalizedTransactionTrace", 9)?;

        let TransactionTrace { action, error, result, subtraces, trace_address } = &self.trace;

        match action {
            Action::Call(call_action) => {
                s.serialize_field("action", call_action)?;
            }
            Action::Create(create_action) => {
                s.serialize_field("action", create_action)?;
            }
            Action::Selfdestruct(selfdestruct_action) => {
                s.serialize_field("action", selfdestruct_action)?;
            }
            Action::Reward(reward_action) => {
                s.serialize_field("action", reward_action)?;
            }
        }
        if let Some(block_hash) = self.block_hash {
            s.serialize_field("blockHash", &block_hash)?;
        }
        if let Some(block_number) = self.block_number {
            s.serialize_field("blockNumber", &block_number)?;
        }

        if let Some(error) = error {
            s.serialize_field("error", error)?;
        }
        match result {
            Some(TraceOutput::Call(call)) => s.serialize_field("result", call)?,
            Some(TraceOutput::Create(create)) => s.serialize_field("result", create)?,
            None => s.serialize_field("result", &None::<()>)?,
        }

        s.serialize_field("subtraces", &subtraces)?;
        s.serialize_field("traceAddress", &trace_address)?;

        if let Some(transaction_hash) = &self.transaction_hash {
            s.serialize_field("transactionHash", transaction_hash)?;
        }
        if let Some(transaction_position) = &self.transaction_position {
            s.serialize_field("transactionPosition", transaction_position)?;
        }

        s.serialize_field("type", &action.kind())?;

        s.end()
    }
}

/// A record of a full VM trace for a CALL/CREATE.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmTrace {
    /// The code to be executed.
    pub code: Bytes,
    /// All executed instructions.
    pub ops: Vec<VmInstruction>,
}

/// A record of a single VM instruction, opcode level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmInstruction {
    /// The gas cost for this instruction.
    pub cost: u64,
    /// Information concerning the execution of the operation.
    pub ex: Option<VmExecutedOperation>,
    /// The program counter.
    pub pc: usize,
    /// Subordinate trace of the CALL/CREATE if applicable.
    pub sub: Option<VmTrace>,
    /// Stringified opcode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// Index of the instruction in the set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idx: Option<String>,
}

/// A record of an executed VM operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmExecutedOperation {
    /// The total gas used.
    pub used: u64,
    /// The stack item placed, if any.
    pub push: Vec<U256>,
    /// If altered, the memory delta.
    pub mem: Option<MemoryDelta>,
    /// The altered storage value, if any.
    pub store: Option<StorageDelta>,
}

/// A diff of some chunk of memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDelta {
    /// Offset into memory the change begins.
    pub off: usize,
    /// The changed data.
    pub data: Bytes,
}

/// A diff of some storage value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDelta {
    /// Storage key.
    pub key: U256,
    /// Storage value belonging to the key.
    pub val: U256,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use similar_asserts::assert_eq;
    use std::str::FromStr;

    #[test]
    fn test_transaction_trace() {
        let s = r#"{
            "action": {
                "from": "0x66e29f0b6b1b07071f2fde4345d512386cb66f5f",
                "callType": "call",
                "gas": "0x10bfc",
                "input": "0xf6cd1e8d0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000011c37937e080000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ec6952892271c8ee13f12e118484e03149281c9600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000010480862479000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000002000000000000000000000000160f5f00288e9e1cc8655b327e081566e580a71d00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000011c37937e080000fffffffffffffffffffffffffffffffffffffffffffffffffee3c86c81f8000000000000000000000000000000000000000000000000000000000000",
                "to": "0x160f5f00288e9e1cc8655b327e081566e580a71d",
                "value": "0x244b"
            },
            "error": "Reverted",
            "result": {
                "gasUsed": "0x9daf",
                "output": "0x000000000000000000000000000000000000000000000000011c37937e080000"
            },
            "subtraces": 3,
            "traceAddress": [],
            "type": "call"
        }"#;
        let val = serde_json::from_str::<TransactionTrace>(s).unwrap();
        serde_json::to_value(val).unwrap();
    }

    #[test]
    fn test_selfdestruct_suicide() {
        let input = r#"{
            "action": {
                "address": "0x66e29f0b6b1b07071f2fde4345d512386cb66f5f",
                "refundAddress": "0x66e29f0b6b1b07071f2fde4345d512386cb66f5f",
                "balance": "0x244b"
            },
            "error": "Reverted",
            "result": {
                "gasUsed": "0x9daf",
                "output": "0x000000000000000000000000000000000000000000000000011c37937e080000"
            },
            "subtraces": 3,
            "traceAddress": [],
            "type": "suicide"
        }"#;
        let val = serde_json::from_str::<TransactionTrace>(input).unwrap();
        assert!(val.action.is_selfdestruct());

        let json = serde_json::to_value(val.clone()).unwrap();
        let expect = serde_json::from_str::<serde_json::Value>(input).unwrap();
        assert_eq!(json, expect);
        let s = serde_json::to_string(&val).unwrap();
        let json = serde_json::from_str::<serde_json::Value>(&s).unwrap();
        assert_eq!(json, expect);

        let input = input.replace("suicide", "selfdestruct");
        let val = serde_json::from_str::<TransactionTrace>(&input).unwrap();
        assert!(val.action.is_selfdestruct());
    }

    #[derive(Debug)]
    struct TraceTestCase {
        trace: LocalizedTransactionTrace,
        expected_json: Value,
    }

    #[test]
    fn test_serialization_order() {
        let test_cases = vec![
            TraceTestCase {
                trace: LocalizedTransactionTrace {
                    trace: TransactionTrace {
                        action: Action::Call(CallAction {
                            from: "0x4f4495243837681061c4743b74b3eedf548d56a5".parse::<Address>().unwrap(),
                            call_type: CallType::DelegateCall,
                            gas: 3148955,
                            input: Bytes::from_str("0x585a9fd40000000000000000000000000000000000000000000000000000000000000040a47c5ad9a4af285720eae6cc174a9c75c5bbaf973b00f1a0c191327445b6581000000000000000000000000000000000000000000000000000000000000001e000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000140000000000000000000000000ce16f69375520ab01377ce7b88f5ba8c48f8d666f61490331372e432315cd97447e3bc452d6c73a6e0536260a88ddab46f85c88d00000000000000000000000000000000000000000000000000000000000001a0000000000000000000000000000000000000000000000000000000000aab8cf0fbfb038751339cb61161fa11789b41a78f1b7b0e12cf8e467d403590b7a5f26f0000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000646616e746f6d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002a3078636531364636393337353532306162303133373763653742383866354241384334384638443636360000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000045553444300000000000000000000000000000000000000000000000000000000").unwrap(),
                            to:  "0x99b5fa03a5ea4315725c43346e55a6a6fbd94098".parse::<Address>().unwrap(),
                            value: U256::from(0),
                        }),
                        error: None,
                        result: Some(TraceOutput::Call(CallOutput { gas_used: 32364, output: Bytes::new() })),
                        subtraces: 0,
                        trace_address: vec![0, 10, 0],
                    },
                    block_hash: Some(B256::ZERO),
                    block_number: Some(18557272),
                    transaction_hash: Some(B256::from_str("0x54160ddcdbfaf98a43a43c328ebd44aa99faa765e0daa93e61145b06815a4071").unwrap()),
                    transaction_position: Some(102),
                },
                expected_json: json!({
                    "action": {
                        "from": "0x4f4495243837681061c4743b74b3eedf548d56a5",
                        "callType": "delegatecall",
                        "gas": "0x300c9b",
                        "input": "0x585a9fd40000000000000000000000000000000000000000000000000000000000000040a47c5ad9a4af285720eae6cc174a9c75c5bbaf973b00f1a0c191327445b6581000000000000000000000000000000000000000000000000000000000000001e000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000140000000000000000000000000ce16f69375520ab01377ce7b88f5ba8c48f8d666f61490331372e432315cd97447e3bc452d6c73a6e0536260a88ddab46f85c88d00000000000000000000000000000000000000000000000000000000000001a0000000000000000000000000000000000000000000000000000000000aab8cf0fbfb038751339cb61161fa11789b41a78f1b7b0e12cf8e467d403590b7a5f26f0000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000646616e746f6d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002a3078636531364636393337353532306162303133373763653742383866354241384334384638443636360000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000045553444300000000000000000000000000000000000000000000000000000000",
                        "to": "0x99b5fa03a5ea4315725c43346e55a6a6fbd94098",
                        "value": "0x0"
                    },
                    "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "blockNumber": 18557272,
                    "result": {
                        "gasUsed": "0x7e6c",
                        "output": "0x"
                    },
                    "subtraces": 0,
                    "traceAddress": [
                        0,
                        10,
                        0
                    ],
                    "transactionHash": "0x54160ddcdbfaf98a43a43c328ebd44aa99faa765e0daa93e61145b06815a4071",
                    "transactionPosition": 102,
                    "type": "call"
                }),
            },
            TraceTestCase {
                trace: LocalizedTransactionTrace {
                    trace: TransactionTrace {
                        action: Action::Create(CreateAction{
                            from: "0x4f4495243837681061c4743b74b3eedf548d56a5".parse::<Address>().unwrap(),
                            creation_method: CreationMethod::Create,
                            gas: 3438907,
                            init: Bytes::from_str("0x6080604052600160005534801561001557600080fd5b50610324806100256000396000f3fe608060405234801561001057600080fd5b50600436106100355760003560e01c8062f55d9d1461003a5780631cff79cd1461004f575b600080fd5b61004d6100483660046101da565b610079565b005b61006261005d3660046101fc565b6100bb565b60405161007092919061027f565b60405180910390f35b6002600054141561009d5760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff8116ff5b60006060600260005414156100e35760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff85163b610136576040517f6f7c43f100000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8473ffffffffffffffffffffffffffffffffffffffff16848460405161015d9291906102de565b6000604051808303816000865af19150503d806000811461019a576040519150601f19603f3d011682016040523d82523d6000602084013e61019f565b606091505b50600160005590969095509350505050565b803573ffffffffffffffffffffffffffffffffffffffff811681146101d557600080fd5b919050565b6000602082840312156101ec57600080fd5b6101f5826101b1565b9392505050565b60008060006040848603121561021157600080fd5b61021a846101b1565b9250602084013567ffffffffffffffff8082111561023757600080fd5b818601915086601f83011261024b57600080fd5b81358181111561025a57600080fd5b87602082850101111561026c57600080fd5b6020830194508093505050509250925092565b821515815260006020604081840152835180604085015260005b818110156102b557858101830151858201606001528201610299565b818111156102c7576000606083870101525b50601f01601f191692909201606001949350505050565b818382376000910190815291905056fea264697066735822122032cb5e746816b7fac95205c068b30da37bd40119a57265be331c162cae74712464736f6c63430008090033").unwrap(),
                            value: U256::from(0),
                        }),
                        error: None,
                        result: Some(TraceOutput::Create(CreateOutput { gas_used: 183114, address: "0x7eb6c6c1db08c0b9459a68cfdcedab64f319c138".parse::<Address>().unwrap(), code: Bytes::from_str("0x608060405234801561001057600080fd5b50600436106100355760003560e01c8062f55d9d1461003a5780631cff79cd1461004f575b600080fd5b61004d6100483660046101da565b610079565b005b61006261005d3660046101fc565b6100bb565b60405161007092919061027f565b60405180910390f35b6002600054141561009d5760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff8116ff5b60006060600260005414156100e35760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff85163b610136576040517f6f7c43f100000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8473ffffffffffffffffffffffffffffffffffffffff16848460405161015d9291906102de565b6000604051808303816000865af19150503d806000811461019a576040519150601f19603f3d011682016040523d82523d6000602084013e61019f565b606091505b50600160005590969095509350505050565b803573ffffffffffffffffffffffffffffffffffffffff811681146101d557600080fd5b919050565b6000602082840312156101ec57600080fd5b6101f5826101b1565b9392505050565b60008060006040848603121561021157600080fd5b61021a846101b1565b9250602084013567ffffffffffffffff8082111561023757600080fd5b818601915086601f83011261024b57600080fd5b81358181111561025a57600080fd5b87602082850101111561026c57600080fd5b6020830194508093505050509250925092565b821515815260006020604081840152835180604085015260005b818110156102b557858101830151858201606001528201610299565b818111156102c7576000606083870101525b50601f01601f191692909201606001949350505050565b818382376000910190815291905056fea264697066735822122032cb5e746816b7fac95205c068b30da37bd40119a57265be331c162cae74712464736f6c63430008090033").unwrap() })),
                        subtraces: 0,
                        trace_address: vec![0, 7, 0, 0],
                    },
                    block_hash: Some(B256::from_str("0xd5ac5043011d4f16dba7841fa760c4659644b78f663b901af4673b679605ed0d").unwrap()),
                    block_number: Some(18557272),
                    transaction_hash: Some(B256::from_str("0x54160ddcdbfaf98a43a43c328ebd44aa99faa765e0daa93e61145b06815a4071").unwrap()),
                    transaction_position: Some(102),
                },
                expected_json: json!({
                    "action": {
                        "from": "0x4f4495243837681061c4743b74b3eedf548d56a5",
                        "creationMethod": "create",
                        "gas": "0x34793b",
                        "init": "0x6080604052600160005534801561001557600080fd5b50610324806100256000396000f3fe608060405234801561001057600080fd5b50600436106100355760003560e01c8062f55d9d1461003a5780631cff79cd1461004f575b600080fd5b61004d6100483660046101da565b610079565b005b61006261005d3660046101fc565b6100bb565b60405161007092919061027f565b60405180910390f35b6002600054141561009d5760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff8116ff5b60006060600260005414156100e35760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff85163b610136576040517f6f7c43f100000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8473ffffffffffffffffffffffffffffffffffffffff16848460405161015d9291906102de565b6000604051808303816000865af19150503d806000811461019a576040519150601f19603f3d011682016040523d82523d6000602084013e61019f565b606091505b50600160005590969095509350505050565b803573ffffffffffffffffffffffffffffffffffffffff811681146101d557600080fd5b919050565b6000602082840312156101ec57600080fd5b6101f5826101b1565b9392505050565b60008060006040848603121561021157600080fd5b61021a846101b1565b9250602084013567ffffffffffffffff8082111561023757600080fd5b818601915086601f83011261024b57600080fd5b81358181111561025a57600080fd5b87602082850101111561026c57600080fd5b6020830194508093505050509250925092565b821515815260006020604081840152835180604085015260005b818110156102b557858101830151858201606001528201610299565b818111156102c7576000606083870101525b50601f01601f191692909201606001949350505050565b818382376000910190815291905056fea264697066735822122032cb5e746816b7fac95205c068b30da37bd40119a57265be331c162cae74712464736f6c63430008090033",
                        "value": "0x0"
                    },
                    "blockHash": "0xd5ac5043011d4f16dba7841fa760c4659644b78f663b901af4673b679605ed0d",
                    "blockNumber": 18557272,
                    "result": {
                        "address": "0x7eb6c6c1db08c0b9459a68cfdcedab64f319c138",
                        "code": "0x608060405234801561001057600080fd5b50600436106100355760003560e01c8062f55d9d1461003a5780631cff79cd1461004f575b600080fd5b61004d6100483660046101da565b610079565b005b61006261005d3660046101fc565b6100bb565b60405161007092919061027f565b60405180910390f35b6002600054141561009d5760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff8116ff5b60006060600260005414156100e35760405163caa30f5560e01b815260040160405180910390fd5b600260005573ffffffffffffffffffffffffffffffffffffffff85163b610136576040517f6f7c43f100000000000000000000000000000000000000000000000000000000815260040160405180910390fd5b8473ffffffffffffffffffffffffffffffffffffffff16848460405161015d9291906102de565b6000604051808303816000865af19150503d806000811461019a576040519150601f19603f3d011682016040523d82523d6000602084013e61019f565b606091505b50600160005590969095509350505050565b803573ffffffffffffffffffffffffffffffffffffffff811681146101d557600080fd5b919050565b6000602082840312156101ec57600080fd5b6101f5826101b1565b9392505050565b60008060006040848603121561021157600080fd5b61021a846101b1565b9250602084013567ffffffffffffffff8082111561023757600080fd5b818601915086601f83011261024b57600080fd5b81358181111561025a57600080fd5b87602082850101111561026c57600080fd5b6020830194508093505050509250925092565b821515815260006020604081840152835180604085015260005b818110156102b557858101830151858201606001528201610299565b818111156102c7576000606083870101525b50601f01601f191692909201606001949350505050565b818382376000910190815291905056fea264697066735822122032cb5e746816b7fac95205c068b30da37bd40119a57265be331c162cae74712464736f6c63430008090033",
                        "gasUsed": "0x2cb4a"
                    },
                    "subtraces": 0,
                    "traceAddress": [
                        0,
                        7,
                        0,
                        0
                    ],
                    "transactionHash": "0x54160ddcdbfaf98a43a43c328ebd44aa99faa765e0daa93e61145b06815a4071",
                    "transactionPosition": 102,
                    "type": "create"
                }),
            }
        ];

        for (i, test_case) in test_cases.iter().enumerate() {
            let serialized = serde_json::to_string(&test_case.trace).unwrap();
            let actual_json: Value = serde_json::from_str(&serialized).unwrap();

            assert_eq!(
                actual_json, test_case.expected_json,
                "Test case {} failed; Trace: {:?}",
                i, test_case.trace
            );
        }
    }

    #[test]
    fn test_deserialize_serialize() {
        let reference_data = r#"{
  "action": {
    "from": "0xc77820eef59629fc8d88154977bc8de8a1b2f4ae",
    "callType": "call",
    "gas": "0x4a0d00",
    "input": "0x12",
    "to": "0x4f4495243837681061c4743b74b3eedf548d56a5",
    "value": "0x0"
  },
  "blockHash": "0xd5ac5043011d4f16dba7841fa760c4659644b78f663b901af4673b679605ed0d",
  "blockNumber": 18557272,
  "result": {
    "gasUsed": "0x17d337",
    "output": "0x"
  },
  "subtraces": 1,
  "traceAddress": [],
  "transactionHash": "0x54160ddcdbfaf98a43a43c328ebd44aa99faa765e0daa93e61145b06815a4071",
  "transactionPosition": 102,
  "type": "call"
}"#;

        let trace: LocalizedTransactionTrace = serde_json::from_str(reference_data).unwrap();
        assert!(trace.trace.action.is_call());
        let serialized = serde_json::to_string_pretty(&trace).unwrap();
        assert_eq!(serialized, reference_data);
    }

    #[test]
    fn test_deserialize_serialize_selfdestruct() {
        let reference_data = r#"{
  "action": {
    "address": "0xc77820eef59629fc8d88154977bc8de8a1b2f4ae",
    "balance": "0x0",
    "refundAddress": "0x4f4495243837681061c4743b74b3eedf548d56a5"
  },
  "blockHash": "0xd5ac5043011d4f16dba7841fa760c4659644b78f663b901af4673b679605ed0d",
  "blockNumber": 18557272,
  "result": {
    "gasUsed": "0x17d337",
    "output": "0x"
  },
  "subtraces": 1,
  "traceAddress": [],
  "transactionHash": "0x54160ddcdbfaf98a43a43c328ebd44aa99faa765e0daa93e61145b06815a4071",
  "transactionPosition": 102,
  "type": "suicide"
}"#;

        let trace: LocalizedTransactionTrace = serde_json::from_str(reference_data).unwrap();
        assert!(trace.trace.action.is_selfdestruct());
        let serialized = serde_json::to_string_pretty(&trace).unwrap();
        assert_eq!(serialized, reference_data);
    }

    #[test]
    fn test_transaction_trace_null_result() {
        let trace = TransactionTrace {
            action: Action::Call(CallAction {
                from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
                call_type: CallType::Call,
                gas: 100000,
                input: Bytes::from_str("0x1234").unwrap(),
                to: Address::from_str("0x0987654321098765432109876543210987654321").unwrap(),
                value: U256::from(0),
            }),
            ..Default::default()
        };

        let serialized = serde_json::to_string(&trace).unwrap();
        let deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized["result"], serde_json::Value::Null);
        assert!(deserialized.as_object().unwrap().contains_key("result"));
        assert!(!deserialized.as_object().unwrap().contains_key("error"));

        let deserialized_trace: TransactionTrace = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized_trace.result, None);
    }

    #[test]
    fn test_transaction_trace_error_result() {
        let trace = TransactionTrace { error: Some("Reverted".to_string()), ..Default::default() };

        let serialized = serde_json::to_string(&trace).unwrap();
        let deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized["result"], serde_json::Value::Null);
        assert!(deserialized.as_object().unwrap().contains_key("result"));
        assert!(deserialized.as_object().unwrap().contains_key("error"));

        let deserialized_trace: TransactionTrace = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized_trace.result, None);
    }

    #[test]
    fn test_nethermind_trace_result_null_output_value() {
        let reference_data = r#"{
  "output": null,
  "stateDiff": {
    "0x5e1d1eb61e1164d5a50b28c575da73a29595dff7": {
      "balance": "=",
      "code": "=",
      "nonce": "=",
      "storage": {
        "0x0000000000000000000000000000000000000000000000000000000000000005": {
          "*": {
            "from": "0x0000000000000000000000000000000000000000000000000000000000042f66",
            "to": "0x0000000000000000000000000000000000000000000000000000000000042f67"
          }
        }
      }
    }
  },
  "trace": [],
  "vmTrace": null,
  "transactionHash": "0xe56a5e7455c45b1842b35dbcab9d024b21870ee59820525091e183b573b4f9eb"
}"#;
        let trace =
            serde_json::from_str::<TraceResultsWithTransactionHash>(reference_data).unwrap();
        assert_eq!(trace.full_trace.output, Bytes::default());
    }
}
