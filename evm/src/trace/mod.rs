use crate::{
    abi::CHEATCODE_ADDRESS,
    debug::Instruction,
    executor::{fork::CreateFork, opts::EvmOpts, Backend, Executor, ExecutorBuilder},
    trace::identifier::LocalTraceIdentifier,
    utils::evm_spec,
    CallKind,
};
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};
use ethers::{
    abi::{ethereum_types::BigEndianHash, Address, RawLog},
    core::utils::to_checksum,
    solc::EvmVersion,
    types::{Bytes, DefaultFrame, GethDebugTracingOptions, StructLog, H256, U256},
};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_config::Config;
use hashbrown::HashMap;
use node::CallTraceNode;
use revm::{
    interpreter::{opcode, CallContext, InstructionResult, Memory, Stack},
    primitives::Env,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt::{self, Write},
    ops::{Deref, DerefMut},
};
use yansi::{Color, Paint};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;

mod decoder;
pub mod node;
pub mod utils;

/// a default executor with tracing enabled
pub struct TracingExecutor {
    executor: Executor,
}

impl TracingExecutor {
    pub async fn new(
        env: revm::primitives::Env,
        fork: Option<CreateFork>,
        version: Option<EvmVersion>,
        debug: bool,
    ) -> Self {
        let db = Backend::spawn(fork).await;

        // configures a bare version of the evm executor: no cheatcode inspector is enabled,
        // tracing will be enabled only for the targeted transaction
        let builder = ExecutorBuilder::default()
            .with_config(env)
            .with_spec(evm_spec(&version.unwrap_or_default()));

        let mut executor = builder.build(db);

        executor.set_tracing(true).set_debugger(debug);

        Self { executor }
    }

    /// uses the fork block number from the config
    pub async fn get_fork_material(
        config: &Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<(Env, Option<CreateFork>, Option<ethers::types::Chain>)> {
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;

        let env = evm_opts.evm_env().await?;

        let fork = evm_opts.get_fork(config, env.clone());

        Ok((env, fork, evm_opts.get_remote_chain_id()))
    }
}

impl Deref for TracingExecutor {
    type Target = Executor;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

impl DerefMut for TracingExecutor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.executor
    }
}

pub type Traces = Vec<(TraceKind, CallTraceArena)>;

/// An arena of [CallTraceNode]s
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallTraceArena {
    /// The arena of nodes
    pub arena: Vec<CallTraceNode>,
}

impl Default for CallTraceArena {
    fn default() -> Self {
        CallTraceArena { arena: vec![Default::default()] }
    }
}

impl CallTraceArena {
    /// Pushes a new trace into the arena, returning the trace ID
    pub fn push_trace(&mut self, entry: usize, new_trace: CallTrace) -> usize {
        match new_trace.depth {
            // The entry node, just update it
            0 => {
                self.arena[0].trace = new_trace;
                0
            }
            // We found the parent node, add the new trace as a child
            _ if self.arena[entry].trace.depth == new_trace.depth - 1 => {
                let id = self.arena.len();

                let trace_location = self.arena[entry].children.len();
                self.arena[entry].ordering.push(LogCallOrder::Call(trace_location));
                let node = CallTraceNode {
                    parent: Some(entry),
                    trace: new_trace,
                    idx: id,
                    ..Default::default()
                };
                self.arena.push(node);
                self.arena[entry].children.push(id);

                id
            }
            // We haven't found the parent node, go deeper
            _ => self.push_trace(
                *self.arena[entry].children.last().expect("Disconnected trace"),
                new_trace,
            ),
        }
    }

    pub fn addresses(&self) -> HashSet<(&Address, Option<&[u8]>)> {
        self.arena
            .iter()
            .map(|node| {
                if node.trace.created() {
                    if let RawOrDecodedReturnData::Raw(ref bytes) = node.trace.output {
                        return (&node.trace.address, Some(bytes.as_ref()));
                    }
                }

                (&node.trace.address, None)
            })
            .collect()
    }

    // Recursively fill in the geth trace by going through the traces
    fn add_to_geth_trace(
        &self,
        storage: &mut HashMap<Address, BTreeMap<H256, H256>>,
        trace_node: &CallTraceNode,
        struct_logs: &mut Vec<StructLog>,
        opts: &GethDebugTracingOptions,
    ) {
        let mut child_id = 0;
        // Iterate over the steps inside the given trace
        for step in trace_node.trace.steps.iter() {
            let mut log: StructLog = step.into();

            // Fill in memory and storage depending on the options
            if !opts.disable_storage.unwrap_or_default() {
                let contract_storage = storage.entry(step.contract).or_default();
                if let Some((key, value)) = step.state_diff {
                    contract_storage.insert(H256::from_uint(&key), H256::from_uint(&value));
                    log.storage = Some(contract_storage.clone());
                }
            }
            if opts.disable_stack.unwrap_or_default() {
                log.stack = None;
            }
            if !opts.enable_memory.unwrap_or_default() {
                log.memory = None;
            }

            // Add step to geth trace
            struct_logs.push(log);

            // Check if the step was a call
            match step.op {
                Instruction::OpCode(opc) => {
                    match opc {
                        // If yes, descend into a child trace
                        opcode::CREATE
                        | opcode::CREATE2
                        | opcode::DELEGATECALL
                        | opcode::CALL
                        | opcode::STATICCALL
                        | opcode::CALLCODE => {
                            self.add_to_geth_trace(
                                storage,
                                &self.arena[trace_node.children[child_id]],
                                struct_logs,
                                opts,
                            );
                            child_id += 1;
                        }
                        _ => {}
                    }
                }
                Instruction::Cheatcode(_) => {}
            }
        }
    }

    /// Generate a geth-style trace e.g. for debug_traceTransaction
    pub fn geth_trace(
        &self,
        receipt_gas_used: U256,
        opts: GethDebugTracingOptions,
    ) -> DefaultFrame {
        if self.arena.is_empty() {
            return Default::default();
        }

        let mut storage = HashMap::new();
        // Fetch top-level trace
        let main_trace_node = &self.arena[0];
        let main_trace = &main_trace_node.trace;
        // Start geth trace
        let mut acc = DefaultFrame {
            // If the top-level trace succeeded, then it was a success
            failed: !main_trace.success,
            gas: receipt_gas_used,
            return_value: main_trace.output.to_bytes(),
            ..Default::default()
        };

        self.add_to_geth_trace(&mut storage, main_trace_node, &mut acc.struct_logs, &opts);

        acc
    }
}

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

impl fmt::Display for CallTraceArena {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn inner(
            arena: &CallTraceArena,
            writer: &mut (impl Write + ?Sized),
            idx: usize,
            left: &str,
            child: &str,
            verbose: bool,
        ) -> fmt::Result {
            let node = &arena.arena[idx];

            // Display trace header
            if !verbose {
                writeln!(writer, "{left}{}", node.trace)?;
            } else {
                writeln!(writer, "{left}{:#}", node.trace)?;
            }

            // Display logs and subcalls
            let left_prefix = format!("{child}{BRANCH}");
            let right_prefix = format!("{child}{PIPE}");
            for child in &node.ordering {
                match child {
                    LogCallOrder::Log(index) => {
                        let mut log = String::new();
                        write!(log, "{}", node.logs[*index])?;

                        // Prepend our tree structure symbols to each line of the displayed log
                        log.lines().enumerate().try_for_each(|(i, line)| {
                            writeln!(
                                writer,
                                "{}{}",
                                if i == 0 { &left_prefix } else { &right_prefix },
                                line
                            )
                        })?;
                    }
                    LogCallOrder::Call(index) => {
                        inner(
                            arena,
                            writer,
                            node.children[*index],
                            &left_prefix,
                            &right_prefix,
                            verbose,
                        )?;
                    }
                }
            }

            // Display trace return data
            let color = trace_color(&node.trace);
            write!(writer, "{child}{EDGE}")?;
            write!(writer, "{}", color.paint(RETURN))?;
            if node.trace.created() {
                if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                    writeln!(writer, "{} bytes of code", bytes.len())?;
                } else {
                    unreachable!("We should never have decoded calldata for contract creations");
                }
            } else {
                writeln!(writer, "{}", node.trace.output)?;
            }

            Ok(())
        }

        inner(self, f, 0, "  ", "  ", f.alternate())
    }
}

/// A raw or decoded log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawOrDecodedLog {
    /// A raw log
    Raw(RawLog),
    /// A decoded log.
    ///
    /// The first member of the tuple is the event name, and the second is a vector of decoded
    /// parameters.
    Decoded(String, Vec<(String, String)>),
}

impl fmt::Display for RawOrDecodedLog {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RawOrDecodedLog::Raw(log) => {
                for (i, topic) in log.topics.iter().enumerate() {
                    writeln!(
                        f,
                        "{:>13}: {}",
                        if i == 0 { "emit topic 0".to_string() } else { format!("topic {i}") },
                        Paint::cyan(format!("0x{}", hex::encode(topic)))
                    )?;
                }

                write!(
                    f,
                    "          data: {}",
                    Paint::cyan(format!("0x{}", hex::encode(&log.data)))
                )
            }
            RawOrDecodedLog::Decoded(name, params) => {
                let params = params
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect::<Vec<String>>()
                    .join(", ");

                write!(f, "emit {}({params})", Paint::cyan(name.clone()))
            }
        }
    }
}

/// Ordering enum for calls and logs
///
/// i.e. if Call 0 occurs before Log 0, it will be pushed into the `CallTraceNode`'s ordering before
/// the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogCallOrder {
    Log(usize),
    Call(usize),
}

/// Raw or decoded calldata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum RawOrDecodedCall {
    /// Raw calldata
    Raw(Bytes),
    /// Decoded calldata.
    ///
    /// The first element in the tuple is the function name, second is the function signature and
    /// the third element is a vector of decoded parameters.
    Decoded(String, String, Vec<String>),
}

impl RawOrDecodedCall {
    pub fn to_raw(&self) -> Vec<u8> {
        match self {
            RawOrDecodedCall::Raw(raw) => raw.to_vec(),
            RawOrDecodedCall::Decoded(_, _, _) => {
                vec![]
            }
        }
    }
}

impl Default for RawOrDecodedCall {
    fn default() -> Self {
        RawOrDecodedCall::Raw(Default::default())
    }
}

/// Raw or decoded return data.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum RawOrDecodedReturnData {
    /// Raw return data
    Raw(Bytes),
    /// Decoded return data
    Decoded(String),
}

impl RawOrDecodedReturnData {
    /// Returns the data as [`Bytes`]
    pub fn to_bytes(&self) -> Bytes {
        match self {
            RawOrDecodedReturnData::Raw(raw) => raw.clone(),
            RawOrDecodedReturnData::Decoded(val) => val.as_bytes().to_vec().into(),
        }
    }

    pub fn to_raw(&self) -> Vec<u8> {
        self.to_bytes().to_vec()
    }
}

impl Default for RawOrDecodedReturnData {
    fn default() -> Self {
        RawOrDecodedReturnData::Raw(Default::default())
    }
}

impl fmt::Display for RawOrDecodedReturnData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self {
            RawOrDecodedReturnData::Raw(bytes) => {
                if bytes.is_empty() {
                    write!(f, "()")
                } else {
                    write!(f, "0x{}", hex::encode(bytes))
                }
            }
            RawOrDecodedReturnData::Decoded(decoded) => write!(f, "{}", decoded.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CallTraceStep {
    // Fields filled in `step`
    /// Call depth
    pub depth: u64,
    /// Program counter before step execution
    pub pc: usize,
    /// Opcode to be executed
    pub op: Instruction,
    /// Current contract address
    pub contract: Address,
    /// Stack before step execution
    pub stack: Stack,
    /// Memory before step execution
    pub memory: Memory,
    /// Remaining gas before step execution
    pub gas: u64,
    /// Gas refund counter before step execution
    pub gas_refund_counter: u64,

    // Fields filled in `step_end`
    /// Gas cost of step execution
    pub gas_cost: u64,
    /// Change of the contract state after step execution (effect of the SLOAD/SSTORE instructions)
    pub state_diff: Option<(U256, U256)>,
    /// Error (if any) after step execution
    pub error: Option<String>,
}

impl From<&CallTraceStep> for StructLog {
    fn from(step: &CallTraceStep) -> Self {
        StructLog {
            depth: step.depth,
            error: step.error.clone(),
            gas: step.gas,
            gas_cost: step.gas_cost,
            memory: Some(convert_memory(step.memory.data())),
            op: step.op.to_string(),
            pc: step.pc as u64,
            refund_counter: if step.gas_refund_counter > 0 {
                Some(step.gas_refund_counter)
            } else {
                None
            },
            stack: Some(step.stack.data().iter().copied().map(|data| data.into()).collect()),
            // Filled in `CallTraceArena::geth_trace` as a result of compounding all slot changes
            storage: None,
        }
    }
}

/// A trace of a call.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CallTrace {
    /// The depth of the call
    pub depth: usize,
    /// Whether the call was successful
    pub success: bool,
    /// The name of the contract, if any.
    ///
    /// The format is `"<artifact>:<contract>"` for easy lookup in local contracts.
    ///
    /// This member is not used by the core call tracing functionality (decoding/displaying). The
    /// intended use case is for other components that may want to process traces by specific
    /// contracts (e.g. gas reports).
    pub contract: Option<String>,
    /// The label for the destination address, if any
    pub label: Option<String>,
    /// caller of this call
    pub caller: Address,
    /// The destination address of the call or the address from the created contract
    pub address: Address,
    /// The kind of call this is
    pub kind: CallKind,
    /// The value transferred in the call
    pub value: U256,
    /// The calldata for the call, or the init code for contract creations
    pub data: RawOrDecodedCall,
    /// The return data of the call if this was not a contract creation, otherwise it is the
    /// runtime bytecode of the created contract
    pub output: RawOrDecodedReturnData,
    /// The gas cost of the call
    pub gas_cost: u64,
    /// The status of the trace's call
    pub status: InstructionResult,
    /// call context of the runtime
    pub call_context: Option<CallContext>,
    /// Opcode-level execution steps
    pub steps: Vec<CallTraceStep>,
}

// === impl CallTrace ===

impl CallTrace {
    /// Whether this is a contract creation or not
    pub fn created(&self) -> bool {
        matches!(self.kind, CallKind::Create | CallKind::Create2)
    }
}

impl Default for CallTrace {
    fn default() -> Self {
        Self {
            depth: Default::default(),
            success: Default::default(),
            contract: Default::default(),
            label: Default::default(),
            caller: Default::default(),
            address: Default::default(),
            kind: Default::default(),
            value: Default::default(),
            data: Default::default(),
            output: Default::default(),
            gas_cost: Default::default(),
            status: InstructionResult::Continue,
            call_context: Default::default(),
            steps: Default::default(),
        }
    }
}

impl fmt::Display for CallTrace {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let address = to_checksum(&self.address, None);
        if self.created() {
            write!(
                f,
                "[{}] {}{} {}@{}",
                self.gas_cost,
                Paint::yellow(CALL),
                Paint::yellow("new"),
                self.label.as_ref().unwrap_or(&"<Unknown>".to_string()),
                address
            )?;
        } else {
            let (func, inputs) = match &self.data {
                RawOrDecodedCall::Raw(bytes) => {
                    // We assume that the fallback function (`data.len() < 4`) counts as decoded
                    // calldata
                    assert!(bytes.len() >= 4);
                    (hex::encode(&bytes[0..4]), hex::encode(&bytes[4..]))
                }
                RawOrDecodedCall::Decoded(func, _, inputs) => (func.clone(), inputs.join(", ")),
            };

            let action = match self.kind {
                // do not show anything for CALLs
                CallKind::Call => "",
                CallKind::StaticCall => "[staticcall]",
                CallKind::CallCode => "[callcode]",
                CallKind::DelegateCall => "[delegatecall]",
                _ => unreachable!(),
            };

            let color = trace_color(self);
            write!(
                f,
                "[{}] {}::{}{}({}) {}",
                self.gas_cost,
                color.paint(self.label.as_ref().unwrap_or(&address)),
                color.paint(func),
                if !self.value.is_zero() {
                    format!("{{value: {}}}", self.value)
                } else {
                    "".to_string()
                },
                inputs,
                Paint::yellow(action),
            )?;
        }

        Ok(())
    }
}

/// Specifies the kind of trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceKind {
    Deployment,
    Setup,
    Execution,
}

/// Chooses the color of the trace depending on the destination address and status of the call.
fn trace_color(trace: &CallTrace) -> Color {
    if trace.address == CHEATCODE_ADDRESS {
        Color::Blue
    } else if trace.success {
        Color::Green
    } else {
        Color::Red
    }
}

/// Given a list of traces and artifacts, it returns a map connecting address to abi
pub fn load_contracts(
    traces: Traces,
    known_contracts: Option<&ContractsByArtifact>,
) -> ContractsByAddress {
    if let Some(contracts) = known_contracts {
        let mut local_identifier = LocalTraceIdentifier::new(contracts);
        let mut decoder = CallTraceDecoderBuilder::new().build();
        for (_, trace) in &traces {
            decoder.identify(trace, &mut local_identifier);
        }

        decoder
            .contracts
            .iter()
            .filter_map(|(addr, name)| {
                if let Ok(Some((_, (abi, _)))) = contracts.find_by_name_or_identifier(name) {
                    return Some((*addr, (name.clone(), abi.clone())));
                }
                None
            })
            .collect()
    } else {
        BTreeMap::new()
    }
}

/// creates the memory data in 32byte chunks
/// see <https://github.com/ethereum/go-ethereum/blob/366d2169fbc0e0f803b68c042b77b6b480836dbc/eth/tracers/logger/logger.go#L450-L452>
fn convert_memory(data: &[u8]) -> Vec<String> {
    let mut memory = Vec::with_capacity((data.len() + 31) / 32);
    for idx in (0..data.len()).step_by(32) {
        let len = std::cmp::min(idx + 32, data.len());
        memory.push(hex::encode(&data[idx..len]));
    }
    memory
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_convert_memory() {
        let mut data = vec![0u8; 32];
        assert_eq!(
            convert_memory(&data),
            vec!["0000000000000000000000000000000000000000000000000000000000000000".to_string()]
        );
        data.extend(data.clone());
        assert_eq!(
            convert_memory(&data),
            vec![
                "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                "0000000000000000000000000000000000000000000000000000000000000000".to_string()
            ]
        );
    }
}
