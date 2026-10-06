//! Source-level debugging through SolDB.
//!
//! Turns a recorded [`CallTraceArena`] into a SolDB [`TransactionTrace`], makes SolDB debug
//! info for each contract from Forge's compiler output, and opens a SolDB session.

use alloy_primitives::{
    Address, U256, hex,
    map::{HashMap, HashSet},
};
use eyre::{Result, eyre};
use foundry_common::ContractsByArtifact;
use foundry_compilers::multi::MultiCompilerLanguage;
use foundry_evm_core::ic::{IcPcMap, PcIcMap};
use foundry_evm_traces::{
    CallKind, CallTraceArena, CallTraceStep, TraceMemberOrder, debug::ContractSources,
};
use soldb_core::{
    ContractCreation, ExecutionCall, StepSnapshot, StorageChange, TraceArtifacts,
    TraceCapabilities, TraceStep, TransactionTrace, WordInterner,
};
use soldb_debugger::{CodeGenerator, ContractDebugInfo, StorageLayout};
use soldb_ethdebug::{LegacySource, SourceMapEnvironment, SourceMapProgram};
use soldb_repl::{DebuggerState, Session};
use std::collections::BTreeMap;

pub(crate) fn run(
    arena: &CallTraceArena,
    names: &[Option<String>],
    sources: &ContractSources,
    known_contracts: &ContractsByArtifact,
    via_ir: bool,
) -> Result<()> {
    let code_generator = if via_ir { CodeGenerator::ViaIr } else { CodeGenerator::Legacy };
    let trace = to_soldb_trace(arena);
    let contracts = debug_info(arena, names, sources, known_contracts, code_generator);

    let mut state = DebuggerState::new();
    state.load_trace(trace);
    state.attach_debug_info(contracts);
    let mut session = Session::new(state);
    soldb_tui::run_interactive(&mut session)?;
    Ok(())
}

fn to_soldb_trace(arena: &CallTraceArena) -> TransactionTrace {
    let mut builder = TraceBuilder::default();
    builder.walk(arena, 0, arena.nodes()[0].trace.caller);
    let root = &arena.nodes()[0].trace;
    let capabilities = TraceCapabilities {
        opcode_steps: true,
        stack: true,
        memory: true,
        storage: true,
        storage_diff: true,
        call_trace: true,
        contract_creation: true,
        ..Default::default()
    };
    TransactionTrace {
        tx_hash: None,
        from_addr: address_hex(root.caller),
        to_addr: Some(address_hex(root.address)),
        value: quantity(root.value),
        input_data: hex::encode_prefixed(&root.data),
        gas_used: root.gas_used,
        output: hex::encode_prefixed(&root.output),
        success: root.success,
        error: (!root.success).then(|| "execution reverted".to_string()),
        debug_trace_available: true,
        contract_address: None,
        backend: Some("foundry".to_string()),
        capabilities,
        artifacts: TraceArtifacts {
            calls: builder.calls,
            creations: builder.creations,
            ..Default::default()
        },
        steps: builder.steps,
    }
}

#[derive(Default)]
struct TraceBuilder {
    words: WordInterner,
    steps: Vec<TraceStep>,
    calls: Vec<ExecutionCall>,
    creations: Vec<ContractCreation>,
    call_stack: Vec<usize>,
    /// The slots each storage context has touched so far.
    touched: HashMap<Address, BTreeMap<String, String>>,
}

enum Record {
    Call(usize),
    Creation(usize),
}

impl TraceBuilder {
    fn walk(&mut self, arena: &CallTraceArena, node_idx: usize, msg_sender: Address) {
        let node = &arena.nodes()[node_idx];
        let context = match node.trace.kind {
            CallKind::DelegateCall | CallKind::CallCode => node.trace.caller,
            _ => node.trace.address,
        };
        // The arena starts at depth 0, SolDB at 1 like `debug_traceTransaction`.
        let depth = node.trace.depth as u64 + 1;
        for order in &node.ordering {
            match order {
                TraceMemberOrder::Step(step_idx) => {
                    self.push_step(&node.trace.steps[*step_idx], depth, context);
                }
                TraceMemberOrder::Call(child) => {
                    let child_idx = node.children[*child];
                    let record = self.enter(arena, child_idx, msg_sender);
                    let child_sender = match arena.nodes()[child_idx].trace.kind {
                        CallKind::DelegateCall | CallKind::CallCode => msg_sender,
                        _ => context,
                    };
                    self.walk(arena, child_idx, child_sender);
                    self.exit(arena, child_idx, record);
                }
                TraceMemberOrder::Log(_) => {}
            }
        }
    }

    fn enter(&mut self, arena: &CallTraceArena, idx: usize, msg_sender: Address) -> Record {
        let trace = &arena.nodes()[idx].trace;
        let parent_id = self.call_stack.last().copied();
        let entry_step = Some(self.steps.len());
        let depth = trace.depth as u64 + 1;
        if trace.kind.is_any_create() {
            let id = self.creations.len();
            self.creations.push(ContractCreation {
                id,
                parent_id,
                depth,
                entry_step,
                exit_step: None,
                create_type: trace.kind.to_str().to_string(),
                caller: address_hex(trace.caller),
                address: Some(address_hex(trace.address)),
                value: quantity(trace.value),
                init_code: hex::encode_prefixed(&trace.data),
                gas_limit: trace.gas_limit,
                gas_used: None,
                output: None,
                success: None,
                error: None,
            });
            return Record::Creation(id);
        }
        // For delegate calls the arena keeps the storage context in `caller` and the code in
        // `address`; the real sender is the one preserved from the calling frame.
        let (from, to) = match trace.kind {
            CallKind::DelegateCall | CallKind::CallCode => (msg_sender, trace.caller),
            _ => (trace.caller, trace.address),
        };
        let id = self.calls.len();
        self.calls.push(ExecutionCall {
            id,
            parent_id,
            depth,
            entry_step,
            exit_step: None,
            call_type: trace.kind.to_str().to_string(),
            from: address_hex(from),
            to: address_hex(to),
            bytecode_address: address_hex(trace.address),
            value: quantity(trace.value),
            input: hex::encode_prefixed(&trace.data),
            gas_limit: trace.gas_limit,
            gas_used: None,
            output: None,
            success: None,
            error: None,
        });
        self.call_stack.push(id);
        Record::Call(id)
    }

    fn exit(&mut self, arena: &CallTraceArena, idx: usize, record: Record) {
        let trace = &arena.nodes()[idx].trace;
        let exit_step = Some(self.steps.len());
        match record {
            Record::Call(id) => {
                self.call_stack.pop();
                let call = &mut self.calls[id];
                call.exit_step = exit_step;
                call.success = Some(trace.success);
            }
            Record::Creation(id) => {
                let creation = &mut self.creations[id];
                creation.exit_step = exit_step;
                creation.success = Some(trace.success);
            }
        }
    }

    fn push_step(&mut self, step: &CallTraceStep, depth: u64, context: Address) {
        let op = self.words.intern(step.op.as_str());
        let stack = step
            .stack
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|value| self.words.intern(&quantity(*value)))
            .collect();
        let memory = step
            .memory
            .as_ref()
            .filter(|memory| !memory.is_empty())
            .map(|memory| hex::encode(memory.as_bytes()));
        let mut storage = BTreeMap::new();
        let mut storage_diff = BTreeMap::new();
        if let Some(change) = &step.storage_change {
            let slot = quantity(change.key);
            let value = quantity(change.value);
            let old = self.touched.entry(context).or_default().insert(slot.clone(), value.clone());
            // A re-read of a value the context already holds is not a change; `had_value`
            // carries the pre-state an SSTORE overwrote.
            if old.as_ref() != Some(&value) {
                let before = change.had_value.map(quantity).or(old);
                storage_diff
                    .insert(slot.clone(), StorageChange { before, after: Some(value.clone()) });
            }
            storage.insert(slot, value);
        }
        let error =
            step.status.filter(|status| !status.is_ok()).map(|status| format!("{status:?}"));
        self.steps.push(TraceStep::new(
            step.pc as u64,
            op,
            step.gas_remaining,
            step.gas_cost,
            depth,
            error,
            StepSnapshot::new(stack, memory, storage, storage_diff),
        ));
    }
}

/// Debug info for every contract whose deployed code executed in the trace
fn debug_info(
    arena: &CallTraceArena,
    names: &[Option<String>],
    sources: &ContractSources,
    known_contracts: &ContractsByArtifact,
    code_generator: CodeGenerator,
) -> Vec<ContractDebugInfo> {
    let mut seen = HashSet::<Address>::default();
    let mut contracts = Vec::new();
    for (node, name) in arena.nodes().iter().zip(names) {
        let (Some(name), Some(code)) = (name, &node.trace.bytecode) else { continue };
        if node.trace.kind.is_any_create() || code.is_empty() || !seen.insert(node.trace.address) {
            continue;
        }
        match contract_debug_info(node.trace.address, name, code, sources, known_contracts) {
            Ok(info) => contracts.push(info.with_code_generator(Some(code_generator))),
            Err(error) => {
                let _ = sh_warn!("no source mapping for `{name}`: {error}");
            }
        }
    }
    contracts
}

fn contract_debug_info(
    address: Address,
    name: &str,
    code: &[u8],
    sources: &ContractSources,
    known_contracts: &ContractsByArtifact,
) -> Result<ContractDebugInfo> {
    let Some(artifacts) = sources.artifacts_by_name.get(name) else {
        return Err(eyre!("no compiled artifact with this name"));
    };
    // Several artifacts can share a name (profiles, stale builds); take the one whose
    // instructions match the executed code.
    let executed = IcPcMap::new(code);
    let mut found = None;
    for artifact in artifacts {
        let (Some(pc_ic), Some(source_map)) =
            (&artifact.pc_ic_map_runtime, &artifact.source_map_runtime_raw)
        else {
            continue;
        };
        if same_layout(pc_ic, &executed) {
            found = Some((artifact, source_map));
            break;
        }
    }
    let Some((artifact, source_map)) = found else {
        return Err(eyre!("no compiled artifact matches the executed code; rebuild the project"));
    };
    let Some(build_sources) = sources.sources_by_id.get(&artifact.build_id) else {
        return Err(eyre!("no sources recorded for build `{}`", artifact.build_id));
    };
    let legacy_sources = build_sources
        .iter()
        .filter(|(_, source)| matches!(source.language, MultiCompilerLanguage::Solc(_)))
        .map(|(id, source)| {
            let path = source.path.display().to_string();
            (u64::from(*id), LegacySource { path, contents: Some(source.source.to_string()) })
        })
        .collect();
    let program = SourceMapProgram::from_parts(
        name,
        SourceMapEnvironment::Runtime,
        source_map,
        code,
        legacy_sources,
        None,
    )
    .map_err(|error| eyre!("{error}"))?;

    let storage_layout = if let Some((_, contract)) =
        known_contracts.iter().find(|(id, _)| id.name == name && id.build_id == artifact.build_id)
        && let Some(layout) = &contract.storage_layout
    {
        let layout = serde_json::to_value(layout.as_ref())?;
        let layout = StorageLayout::parse(&layout)
            .map_err(|error| eyre!("invalid storage layout: {error}"))?;
        Some(layout)
    } else {
        None
    };

    let address = address_hex(address);
    Ok(ContractDebugInfo::new(Some(&address), name, program.info, program.source_contents)
        .with_storage_layout(storage_layout))
}

/// Whether the artifact's code has the same instructions at the same offsets as `executed`.
fn same_layout(artifact: &PcIcMap, executed: &IcPcMap) -> bool {
    if artifact.len() != executed.len() {
        return false;
    }
    for (ic, pc) in executed.iter() {
        if artifact.get(*pc) != Some(*ic) {
            return false;
        }
    }
    true
}

fn address_hex(address: Address) -> String {
    format!("{address:#x}")
}

fn quantity(value: U256) -> String {
    format!("{value:#x}")
}
