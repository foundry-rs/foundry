//! [soldb](https://github.com/walnuthq/soldb) frontend.
//!
//! Converts a recorded call trace into a soldb transaction trace and the local artifacts' source
//! maps into soldb debug info, then runs a soldb session over them or writes them as a bundle that
//! soldb's own tools read.

use alloy_primitives::{Address, Bytes, U256, hex, map::HashSet};
use dialoguer::{Select, console::Term};
use eyre::{OptionExt, Result};
use foundry_common::{
    ContractsByArtifact, Shell,
    fs::{self, write_json_file},
};
use foundry_compilers::{
    artifacts::sourcemap::{Jump, SourceElement, SourceMap},
    multi::MultiCompilerLanguage,
};
use foundry_evm_core::{Breakpoints, ic::PcIcMap};
use foundry_evm_traces::{
    CallTraceArena, CallTraceNode, CallTraceStep, TraceMemberOrder,
    debug::{ArtifactData, ContractSources, SourceData},
};
use foundry_tui::tui_mode;
use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};
use serde_json::{Map, Value, json};
use soldb_core::{
    ContractCreation, ExecutionCall, StepSnapshot, StorageChange, TraceArtifacts,
    TraceCapabilities, TraceStep, TransactionTrace, WordInterner,
};
use soldb_debugger::{CodeGenerator, ContractDebugInfo, StorageLayout};
use soldb_ethdebug::{EthdebugInfo, Instruction};
use soldb_repl::{BreakpointTarget, DebuggerCommand, DebuggerState, Output, Renderer, Session};
use soldb_tui::Exit;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{self, IsTerminal, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// A soldb session over one recorded call trace.
#[derive(Debug)]
pub(crate) struct SoldbDebugger {
    trace: TransactionTrace,
    contracts: Vec<ContractDebugInfo>,
    /// The program counter of each `vm.breakpoint` location and the steps that run it.
    breakpoints: Vec<(u64, Vec<usize>)>,
}

impl SoldbDebugger {
    /// Converts `arena`, whose node contracts are identified by `contract_names`.
    pub(crate) fn new(
        mut arena: CallTraceArena,
        contract_names: &[Option<String>],
        sources: &ContractSources,
        known_contracts: &ContractsByArtifact,
        breakpoints: &Breakpoints,
    ) -> Self {
        let contracts = executed_programs(&arena, contract_names, sources, known_contracts)
            .iter()
            .filter_map(contract_debug_info)
            .collect();
        let (trace, breakpoints) = transaction_trace(&mut arena, breakpoints);
        let breakpoints = breakpoints
            .into_iter()
            .filter(|(_, steps)| !steps.is_empty())
            .map(|((_, pc), steps)| (pc as u64, steps))
            .collect();
        Self { trace, contracts, breakpoints }
    }

    /// Runs the session.
    ///
    /// If stdin and stdout are terminals, the full-screen view opens first. Then commands are
    /// read from stdin, one per line, until `quit` or the end of the input.
    pub(crate) fn try_run(self) -> Result<()> {
        let mut state = DebuggerState::new();
        state.load_trace(self.trace);
        state.attach_debug_info(self.contracts);
        // soldb breakpoints match a program counter in any contract, so each one is limited to
        // the steps that run the `vm.breakpoint` location.
        for (pc, steps) in self.breakpoints {
            let condition =
                steps.iter().map(|step| format!("step == {step}")).collect::<Vec<_>>().join(" || ");
            state.set_conditional_breakpoint_target(&BreakpointTarget::Pc(pc), Some(&condition));
        }
        let mut session = Session::new(state);
        let renderer = Renderer::new(Shell::get().out_supports_color());
        let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();

        emit(&renderer, vec![session.loaded()])?;
        emit(&renderer, session.initial_stop())?;
        if terminal {
            if open_tui(&mut session)? == Flow::Quit {
                return Ok(());
            }
            sh_eprintln!("Type `help` for the command list, `tui` for the full-screen view.")?;
        }

        let mut line = String::new();
        loop {
            if terminal {
                sh_eprint!("soldb> ")?;
                io::stderr().flush()?;
            }
            line.clear();
            if io::stdin().read_line(&mut line)? == 0 {
                break;
            }
            match emit(&renderer, session.execute(DebuggerCommand::parse(&line)))? {
                Flow::Continue => {}
                Flow::Tui if open_tui(&mut session)? == Flow::Continue => {}
                Flow::Tui | Flow::Quit => break,
            }
        }
        Ok(())
    }
}

/// Selects the call to debug, since soldb debugs one.
///
/// With several calls, asks on an interactive terminal and takes the last one otherwise. The last
/// call is the test or script entry point; earlier ones are deployments and `setUp`.
pub(crate) fn select_call(mut arenas: Vec<CallTraceArena>) -> Result<CallTraceArena> {
    let last = arenas.len().checked_sub(1).ok_or_eyre("debug arena is empty")?;
    let term = Term::stderr();
    let selected = if last == 0 {
        last
    } else if tui_mode().is_interactive() && term.is_term() {
        Select::new()
            .with_prompt("Select a call to debug")
            .items(arenas.iter().map(call_label))
            .default(last)
            .interact_on_opt(&term)?
            .ok_or_eyre("debugger call selection cancelled")?
    } else {
        sh_warn!("soldb debugs one call; showing the last of {} calls", last + 1)?;
        last
    };
    Ok(arenas.swap_remove(selected))
}

/// What the session asks the frontend to do after a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Tui,
    Quit,
}

/// Prints the session's answers.
fn emit(renderer: &Renderer, outputs: Vec<Output>) -> Result<Flow> {
    let mut flow = Flow::Continue;
    for output in outputs {
        match output {
            Output::Tui => flow = Flow::Tui,
            Output::Quit => flow = Flow::Quit,
            _ => {}
        }
        sh_print!("{}", renderer.render(&output))?;
    }
    Ok(flow)
}

/// Opens the full-screen view until the user leaves it.
fn open_tui(session: &mut Session) -> Result<Flow> {
    match soldb_tui::run(session) {
        Ok(Exit::Repl) => Ok(Flow::Continue),
        Ok(Exit::Quit) => Ok(Flow::Quit),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            sh_warn!("cannot open the full-screen view: {error}")?;
            Ok(Flow::Continue)
        }
        Err(error) => Err(error.into()),
    }
}

/// Converts `arena` into a soldb trace: its steps in execution order, with each call and
/// creation recorded at the steps it spans.
///
/// Also returns the steps that run each `breakpoints` location. The steps move out of `arena`, so
/// each one is dropped once converted.
fn transaction_trace(
    arena: &mut CallTraceArena,
    breakpoints: &Breakpoints,
) -> (TransactionTrace, BTreeMap<(Address, usize), Vec<usize>>) {
    let mut builder = TraceBuilder {
        breakpoints: breakpoints.values().map(|location| (*location, Vec::new())).collect(),
        ..Default::default()
    };
    builder.visit(arena.nodes_mut(), 0, None);
    let TraceBuilder { steps, calls, creations, breakpoints, .. } = builder;

    let root = &arena.nodes()[0].trace;
    let create = root.kind.is_any_create();
    let trace = TransactionTrace {
        tx_hash: None,
        from_addr: hex::encode_prefixed(root.caller),
        to_addr: (!create).then(|| hex::encode_prefixed(root.address)),
        value: quantity(root.value),
        input_data: hex::encode_prefixed(&root.data),
        gas_used: root.gas_used,
        output: hex::encode_prefixed(&root.output),
        success: root.success,
        error: status_error(root.status),
        debug_trace_available: true,
        contract_address: create.then(|| hex::encode_prefixed(root.address)),
        backend: Some("foundry".to_string()),
        capabilities: TraceCapabilities {
            opcode_steps: true,
            stack: true,
            memory: true,
            storage: true,
            storage_diff: true,
            call_trace: true,
            contract_creation: true,
            ..Default::default()
        },
        artifacts: TraceArtifacts { calls, creations, ..Default::default() },
        steps,
    };
    (trace, breakpoints)
}

/// Collects the soldb steps, calls, and creations of a trace in execution order.
#[derive(Default)]
struct TraceBuilder {
    steps: Vec<TraceStep>,
    calls: Vec<ExecutionCall>,
    creations: Vec<ContractCreation>,
    /// One shared copy of each distinct stack word and mnemonic.
    words: WordInterner,
    /// The last recorded memory and its hex encoding, shared by the steps that did not change it.
    memory: Option<(Bytes, Arc<str>)>,
    /// The storage of the steps that touch no slot.
    no_storage: Arc<BTreeMap<String, String>>,
    /// The steps that run each `vm.breakpoint` location, as code address and program counter.
    breakpoints: BTreeMap<(Address, usize), Vec<usize>>,
}

impl TraceBuilder {
    /// Records node `idx` and its subcalls. `parent` is the id of the enclosing call.
    fn visit(&mut self, nodes: &mut [CallTraceNode], idx: usize, parent: Option<usize>) {
        let node = &mut nodes[idx];
        let ordering = std::mem::take(&mut node.ordering);
        let children = std::mem::take(&mut node.children);
        let steps = std::mem::take(&mut node.trace.steps);
        let trace = &node.trace;
        let address = trace.address;
        let depth = trace.depth as u64 + 1;
        let entry_step = Some(self.steps.len());
        let error = status_error(trace.status);
        let creation = trace.kind.is_any_create().then_some(self.creations.len());
        let mut call = None;
        if creation.is_some() {
            self.creations.push(ContractCreation {
                id: self.creations.len(),
                parent_id: parent,
                depth,
                entry_step,
                exit_step: None,
                create_type: trace.kind.to_string(),
                caller: hex::encode_prefixed(trace.caller),
                address: Some(hex::encode_prefixed(trace.address)),
                value: quantity(trace.value),
                init_code: hex::encode_prefixed(&trace.data),
                gas_limit: trace.gas_limit,
                gas_used: Some(trace.gas_used),
                output: Some(hex::encode_prefixed(&trace.output)),
                success: Some(trace.success),
                error,
            });
        } else {
            // A delegated call runs `address`'s code against the storage of `caller`.
            let to = if trace.kind.is_delegate() { trace.caller } else { trace.address };
            call = Some(self.calls.len());
            self.calls.push(ExecutionCall {
                id: self.calls.len(),
                parent_id: parent,
                depth,
                entry_step,
                exit_step: None,
                call_type: trace.kind.to_string(),
                from: hex::encode_prefixed(trace.caller),
                to: hex::encode_prefixed(to),
                bytecode_address: hex::encode_prefixed(trace.address),
                value: quantity(trace.value),
                input: hex::encode_prefixed(&trace.data),
                gas_limit: trace.gas_limit,
                gas_used: Some(trace.gas_used),
                output: Some(hex::encode_prefixed(&trace.output)),
                success: Some(trace.success),
                error,
            });
        }

        // The ordering lists the steps in recording order.
        let mut steps = steps.into_iter().peekable();
        for order in ordering {
            match order {
                TraceMemberOrder::Step(_) => {
                    if let Some(step) = steps.next() {
                        if let Some(hits) = self.breakpoints.get_mut(&(address, step.pc)) {
                            hits.push(self.steps.len());
                        }
                        self.push_step(&step, steps.peek(), depth);
                    }
                }
                TraceMemberOrder::Call(child) => {
                    self.visit(nodes, children[child], call.or(parent));
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        let exit_step = Some(self.steps.len());
        if let Some(id) = creation {
            self.creations[id].exit_step = exit_step;
        } else if let Some(id) = call {
            self.calls[id].exit_step = exit_step;
        }
    }

    /// Records `step`. `next` is the following step of the same frame, if any.
    fn push_step(&mut self, step: &CallTraceStep, next: Option<&CallTraceStep>, depth: u64) {
        let stack = step.stack.as_deref().unwrap_or_default();
        let memory = step.memory.as_ref().map(|memory| {
            let memory = memory.as_bytes();
            match &self.memory {
                Some((last, hex)) if last == memory => hex.clone(),
                _ => {
                    let hex = Arc::<str>::from(hex::encode(memory));
                    self.memory = Some((memory.clone(), hex.clone()));
                    hex
                }
            }
        });
        // The slot an `SLOAD` or `SSTORE` touches and the value it reads or writes. The full
        // stack snapshots do not record pushes, but an `SLOAD` leaves its value on top of the
        // next step's stack.
        let touched = match step.op {
            OpCode::SLOAD => stack
                .last()
                .copied()
                .zip(next.and_then(|next| next.stack.as_deref()?.last().copied())),
            OpCode::SSTORE => match stack {
                [.., value, slot] => Some((*slot, *value)),
                _ => None,
            },
            _ => None,
        };
        let (storage, storage_diff) = match touched {
            Some((slot, value)) => {
                let (slot, value) = (quantity(slot), quantity(value));
                let before = step
                    .storage_change
                    .as_ref()
                    .filter(|_| step.op == OpCode::SSTORE)
                    .and_then(|change| change.had_value)
                    .map(quantity);
                let change = StorageChange { before, after: Some(value.clone()) };
                (
                    Arc::new(BTreeMap::from([(slot.clone(), value)])),
                    BTreeMap::from([(slot, change)]),
                )
            }
            None => (self.no_storage.clone(), BTreeMap::new()),
        };

        let snapshot = StepSnapshot {
            stack: stack.iter().map(|word| self.words.intern(&quantity(*word))).collect(),
            memory,
            storage,
            storage_diff,
        };
        self.steps.push(TraceStep::new(
            step.pc as u64,
            self.words.intern(step.op.as_str()),
            step.gas_remaining,
            step.gas_cost,
            depth,
            status_error(step.status),
            snapshot,
        ));
    }
}

/// A program the trace executes and the local artifact that describes it.
struct ExecutedProgram<'a> {
    address: Address,
    name: &'a str,
    code: &'a Bytes,
    /// Whether the program is creation code.
    init: bool,
    artifact: &'a ArtifactData,
    /// Solc indexes source maps by instruction counter, but Vyper indexes by program counter.
    solc: bool,
    source_map: &'a SourceMap,
    pc_ic_map: Option<&'a PcIcMap>,
    /// The sources of the build that compiled the program, by source id.
    build_sources: &'a HashMap<u32, Arc<SourceData>>,
    /// The contract's storage layout, as solc writes it.
    storage_layout: Option<Value>,
}

/// The programs `arena` runs that local artifacts describe, once per address and environment.
fn executed_programs<'a>(
    arena: &'a CallTraceArena,
    contract_names: &'a [Option<String>],
    sources: &'a ContractSources,
    known_contracts: &ContractsByArtifact,
) -> Vec<ExecutedProgram<'a>> {
    let mut seen = HashSet::<(Address, bool)>::default();
    let mut programs = Vec::new();
    for (node, name) in arena.nodes().iter().zip(contract_names) {
        let trace = &node.trace;
        let init = trace.kind.is_any_create();
        if let Some(name) = name
            && let Some(code) = &trace.bytecode
            && seen.insert((trace.address, init))
            && let Some(sources_for_name) = sources.get_sources(name)
            && let Some((artifact, source)) = sources_for_name.into_iter().find(|(artifact, _)| {
                if init {
                    artifact.source_map.is_some()
                } else {
                    artifact.source_map_runtime.is_some()
                }
            })
            && let Some(build_sources) = sources.sources_by_id.get(&artifact.build_id)
        {
            let (source_map, pc_ic_map) = if init {
                (artifact.source_map.as_ref(), artifact.pc_ic_map.as_ref())
            } else {
                (artifact.source_map_runtime.as_ref(), artifact.pc_ic_map_runtime.as_ref())
            };
            let Some(source_map) = source_map else { continue };
            let storage_layout = known_contracts
                .find_by_name_or_identifier(name)
                .ok()
                .flatten()
                .and_then(|(_, contract)| contract.storage_layout.as_deref())
                .and_then(|layout| serde_json::to_value(layout).ok());
            programs.push(ExecutedProgram {
                address: trace.address,
                name,
                code,
                init,
                artifact,
                solc: matches!(source.language, MultiCompilerLanguage::Solc(_)),
                source_map,
                pc_ic_map,
                build_sources,
                storage_layout,
            });
        }
    }
    programs
}

/// Adapts the source map of `program` to its instructions, in the ETHDebug shape soldb reads.
fn contract_debug_info(program: &ExecutedProgram<'_>) -> Option<ContractDebugInfo> {
    let mut instructions = Vec::new();
    let mut used_sources = BTreeSet::new();
    let mut pc = 0;
    while let Some(&byte) = program.code.get(pc) {
        let op = OpCode::new(byte);
        let index = if program.solc {
            program.pc_ic_map.and_then(|map| map.get(pc as u32))
        } else {
            Some(pc as u32)
        };
        if let Some(element) = index.and_then(|index| program.source_map.get(index as usize)) {
            let mnemonic =
                op.map_or_else(|| format!("UNKNOWN(0x{byte:02x})"), |op| op.as_str().to_string());
            instructions.push(Instruction {
                offset: pc as u64,
                operation: json!({ "mnemonic": mnemonic }),
                context: Some(instruction_context(
                    element,
                    program.build_sources,
                    &mut used_sources,
                )),
            });
        }
        pc += 1 + op.map_or(0, |op| op.info().immediate_size() as usize);
    }
    if instructions.is_empty() {
        return None;
    }

    let used_sources = used_sources
        .into_iter()
        .filter_map(|id| program.build_sources.get(&id).map(|source| (u64::from(id), source)))
        .collect::<Vec<_>>();
    let info = EthdebugInfo {
        compilation: Value::Null,
        contract_name: program.name.to_string(),
        environment: if program.init { "create" } else { "call" }.to_string(),
        instructions,
        sources: used_sources
            .iter()
            .map(|(id, source)| (*id, source.path.to_string_lossy().into_owned()))
            .collect(),
        variable_locations: BTreeMap::new(),
    };
    let source_contents =
        used_sources.iter().map(|(id, source)| (*id, source.source.to_string())).collect();
    // The code generator decides whether soldb can infer local variables from the stack.
    let code_generator = program
        .artifact
        .via_ir
        .filter(|_| program.solc)
        .map(|via_ir| if via_ir { CodeGenerator::ViaIr } else { CodeGenerator::Legacy });
    let storage_layout =
        program.storage_layout.as_ref().and_then(|layout| StorageLayout::parse(layout).ok());
    Some(
        ContractDebugInfo::new(
            Some(&hex::encode_prefixed(program.address)),
            program.name,
            info,
            source_contents,
        )
        .with_code_generator(code_generator)
        .with_storage_layout(storage_layout),
    )
}

/// Writes a bundle that soldb's own tools read, for example
/// `soldb profile --trace-file <dir>/trace.json --contracts <dir>/contracts.json`.
///
/// The bundle holds the transaction trace, a contracts mapping, one `combined.json` per contract
/// address with the legacy source maps and bytecode `solc --combined-json` writes, and the sources
/// under their project paths, where soldb looks for them.
pub(crate) fn write_bundle(
    dir: &Path,
    mut arena: CallTraceArena,
    contract_names: &[Option<String>],
    sources: &ContractSources,
    known_contracts: &ContractsByArtifact,
) -> Result<()> {
    let mut contracts = BTreeMap::<Address, CombinedJson>::new();
    let mut written_sources = HashSet::<PathBuf>::default();
    for program in executed_programs(&arena, contract_names, sources, known_contracts) {
        // soldb reads legacy source maps by instruction counter, which Vyper's are not.
        if !program.solc {
            continue;
        }
        let contract = contracts.entry(program.address).or_insert_with(|| {
            let path = |id: &u32| {
                program.build_sources.get(id).map(|source| source.path.to_string_lossy())
            };
            let max_id = program.build_sources.keys().max().copied().unwrap_or_default();
            CombinedJson {
                name: program.name.to_string(),
                key: format!(
                    "{}:{}",
                    path(&program.artifact.file_id).unwrap_or_default(),
                    program.name
                ),
                // Ids without a build source stay unreadable placeholders.
                source_list: (0..=max_id)
                    .map(|id| path(&id).map_or_else(|| format!("<missing-{id}>"), Into::into))
                    .collect(),
                fields: Map::new(),
            }
        });
        let (source_map_key, bytecode_key) =
            if program.init { ("srcmap", "bin") } else { ("srcmap-runtime", "bin-runtime") };
        let source_map = program
            .source_map
            .iter()
            .map(|element| {
                format!(
                    "{}:{}:{}:{}:{}",
                    element.offset(),
                    element.length(),
                    element.index_i32(),
                    element.jump().to_str(),
                    element.modifier_depth()
                )
            })
            .collect::<Vec<_>>()
            .join(";");
        contract.fields.insert(source_map_key.to_string(), source_map.into());
        contract.fields.insert(bytecode_key.to_string(), hex::encode(program.code).into());
        if let Some(layout) = &program.storage_layout {
            contract.fields.insert("storage-layout".to_string(), layout.clone());
        }

        for source in program.build_sources.values() {
            // Only project-relative paths are copied, so nothing is written outside `dir`.
            let relative = source.path.is_relative()
                && source
                    .path
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)));
            if relative && written_sources.insert(source.path.clone()) {
                let path = dir.join(&source.path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, source.source.as_bytes())?;
            }
        }
    }

    let mut mapping = Vec::new();
    for (address, contract) in contracts {
        let address = hex::encode_prefixed(address);
        fs::create_dir_all(dir.join(&address))?;
        write_json_file(
            &dir.join(&address).join("combined.json"),
            &json!({
                "sourceList": contract.source_list,
                "contracts": { contract.key: contract.fields },
            }),
        )?;
        mapping.push(json!({ "address": address, "name": contract.name, "debug_dir": address }));
    }
    write_json_file(&dir.join("contracts.json"), &json!({ "contracts": mapping }))?;
    let (trace, _) = transaction_trace(&mut arena, &Breakpoints::default());
    write_json_file(&dir.join("trace.json"), &trace)?;
    Ok(())
}

/// One contract of a bundle's `combined.json`.
struct CombinedJson {
    name: String,
    /// The contract's key, `<source path>:<name>`.
    key: String,
    source_list: Vec<String>,
    /// The source maps, bytecode, and storage layout, under solc's field names.
    fields: Map<String, Value>,
}

/// The ETHDebug context of one source map element: its source range, if the build has the
/// source, and the `invoke` or `return` marker of a jump into or out of a function.
fn instruction_context(
    element: &SourceElement,
    build_sources: &HashMap<u32, Arc<SourceData>>,
    used_sources: &mut BTreeSet<u32>,
) -> Value {
    let mut context = Map::new();
    context.insert("modifierDepth".to_string(), element.modifier_depth().into());
    // Compiler-generated code points at sources that are not in the build.
    if let Some(index) = element.index()
        && build_sources.contains_key(&index)
    {
        used_sources.insert(index);
        context.insert(
            "code".to_string(),
            json!({
                "source": { "id": index },
                "range": { "offset": element.offset(), "length": element.length() },
            }),
        );
    }
    match element.jump() {
        Jump::In => {
            context.insert("invoke".to_string(), json!({}));
        }
        Jump::Out => {
            context.insert("return".to_string(), json!({}));
        }
        Jump::Regular => {}
    }
    Value::Object(context)
}

/// The root call of `arena` as the selection list shows it, e.g. `CounterTest::test_increment()`.
fn call_label(arena: &CallTraceArena) -> String {
    let trace = &arena.nodes()[0].trace;
    let decoded = trace.decoded.as_deref();
    let contract = decoded
        .and_then(|decoded| decoded.label.clone())
        .unwrap_or_else(|| trace.address.to_string());
    let function = match decoded.and_then(|decoded| decoded.call_data.as_ref()) {
        Some(call_data) => call_data.signature.clone(),
        None if trace.kind.is_any_create() => "constructor".to_string(),
        None => hex::encode_prefixed(trace.data.get(..4).unwrap_or_default()),
    };
    format!("{contract}::{function}")
}

/// The error a failed step or call ended with.
fn status_error(status: Option<InstructionResult>) -> Option<String> {
    status.filter(|status| !status.is_ok()).map(|status| format!("{status:?}"))
}

/// Formats `value` as a JSON-RPC quantity.
fn quantity(value: U256) -> String {
    format!("{value:#x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::artifacts::sourcemap;
    use foundry_evm_traces::{CallKind, CallTrace, DecodedCallData, DecodedCallTrace};
    use soldb_ethdebug::SourceMapEnvironment;

    /// A step as debug mode records it: the full stack, without pushes.
    fn step(pc: usize, op: OpCode, stack: &[u64]) -> CallTraceStep {
        CallTraceStep {
            pc,
            op,
            stack: Some(stack.iter().map(|word| U256::from(*word)).collect()),
            push_stack: None,
            memory: None,
            returndata: Bytes::new(),
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_used: 0,
            gas_cost: 0,
            state_gas_cost: None,
            state_gas_reservoir: None,
            state_gas_spent: 0,
            storage_change: None,
            status: (op == OpCode::STOP).then_some(InstructionResult::Stop),
            immediate_bytes: None,
            decoded: None,
        }
    }

    #[test]
    fn converts_steps_calls_and_storage_in_execution_order() {
        let (caller, proxy, implementation) =
            (Address::repeat_byte(1), Address::repeat_byte(0xa), Address::repeat_byte(0xb));
        let mut arena = CallTraceArena::default();
        {
            let root = &mut arena.nodes_mut()[0];
            root.trace = CallTrace {
                caller,
                address: proxy,
                kind: CallKind::Call,
                success: true,
                steps: vec![
                    step(0, OpCode::PUSH1, &[]),
                    step(2, OpCode::SLOAD, &[0]),
                    step(3, OpCode::DELEGATECALL, &[0x29]),
                    step(4, OpCode::STOP, &[]),
                ],
                ..Default::default()
            };
            root.children.push(1);
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Step(1),
                TraceMemberOrder::Step(2),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(3),
            ];
        }
        // A delegated call records the storage context as its caller.
        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace {
                caller: proxy,
                address: implementation,
                kind: CallKind::DelegateCall,
                depth: 1,
                success: true,
                steps: vec![step(0, OpCode::SSTORE, &[0x2a, 0]), step(1, OpCode::STOP, &[])],
                ..Default::default()
            },
            ordering: vec![TraceMemberOrder::Step(0), TraceMemberOrder::Step(1)],
            ..Default::default()
        });

        let breakpoints = Breakpoints::from_iter([('a', (proxy, 3)), ('b', (implementation, 9))]);
        let (trace, breakpoints) = transaction_trace(&mut arena, &breakpoints);
        assert_eq!(
            breakpoints,
            BTreeMap::from([((proxy, 3), vec![2]), ((implementation, 9), Vec::new())])
        );

        let steps = trace
            .steps
            .iter()
            .map(|step| (step.pc, step.op.to_string(), step.depth))
            .collect::<Vec<_>>();
        assert_eq!(
            steps,
            [
                (0, "PUSH1".to_string(), 1),
                (2, "SLOAD".to_string(), 1),
                (3, "DELEGATECALL".to_string(), 1),
                (0, "SSTORE".to_string(), 2),
                (1, "STOP".to_string(), 2),
                (4, "STOP".to_string(), 1),
            ]
        );
        let storage_diff =
            |index: usize| serde_json::to_value(&trace.steps[index].snapshot.storage_diff).unwrap();
        assert_eq!(storage_diff(1), json!({ "0x0": { "before": null, "after": "0x29" } }));
        assert_eq!(storage_diff(3), json!({ "0x0": { "before": null, "after": "0x2a" } }));
        assert_eq!(storage_diff(4), json!({}));

        let calls = trace
            .artifacts
            .calls
            .iter()
            .map(|call| {
                (
                    call.parent_id,
                    call.entry_step,
                    call.exit_step,
                    call.call_type.as_str(),
                    call.to.as_str(),
                    call.bytecode_address.as_str(),
                )
            })
            .collect::<Vec<_>>();
        let (proxy, implementation) =
            (hex::encode_prefixed(proxy), hex::encode_prefixed(implementation));
        assert_eq!(
            calls,
            [
                (None, Some(0), Some(6), "CALL", proxy.as_str(), proxy.as_str()),
                (
                    Some(0),
                    Some(3),
                    Some(5),
                    "DELEGATECALL",
                    proxy.as_str(),
                    implementation.as_str()
                ),
            ]
        );
        assert_eq!(trace.to_addr.as_deref(), Some(proxy.as_str()));
        assert_eq!(trace.from_addr, hex::encode_prefixed(caller));
    }

    /// `PUSH1 0x80; JUMPDEST; STOP` as contract `C` at `0x0c..0c`. The last source map element
    /// points at a generated source.
    fn executed_c() -> (ContractSources, CallTraceArena, Vec<Option<String>>) {
        const CODE: [u8; 4] = [0x60, 0x80, 0x5b, 0x00];
        let mut sources = ContractSources::default();
        sources.sources_by_id.entry("build".to_string()).or_default().insert(
            0,
            Arc::new(SourceData {
                source: Arc::new("contract C {}".to_string()),
                language: Default::default(),
                path: PathBuf::from("src/C.sol"),
                contract_definitions: Vec::new(),
                debug_scopes: Vec::new(),
            }),
        );
        sources.artifacts_by_name.insert(
            "C".to_string(),
            vec![ArtifactData {
                source_map: None,
                source_map_runtime: Some(sourcemap::parse("0:8:0:-;9:3:0:i;1:2:1:o").unwrap()),
                pc_ic_map: None,
                pc_ic_map_runtime: Some(PcIcMap::new(&CODE)),
                build_id: "build".to_string(),
                file_id: 0,
                via_ir: Some(false),
            }],
        );
        let mut arena = CallTraceArena::default();
        let root = &mut arena.nodes_mut()[0].trace;
        root.address = Address::repeat_byte(0xc);
        root.bytecode = Some(Bytes::from_static(&CODE));
        root.steps = vec![step(0, OpCode::PUSH1, &[]), step(2, OpCode::JUMPDEST, &[0x80])];
        arena.nodes_mut()[0].ordering = vec![TraceMemberOrder::Step(0), TraceMemberOrder::Step(1)];
        (sources, arena, vec![Some("C".to_string())])
    }

    #[test]
    fn adapts_source_maps_to_ethdebug_instructions() {
        let (sources, arena, names) = executed_c();
        let address = Address::repeat_byte(0xc);

        let programs = executed_programs(&arena, &names, &sources, &Default::default());
        let contract = contract_debug_info(&programs[0]).unwrap();

        assert_eq!(contract.address, Some(hex::encode_prefixed(address)));
        assert_eq!(contract.code_generator, Some(CodeGenerator::Legacy));
        assert_eq!(contract.info.environment, "call");
        assert_eq!(contract.info.sources, BTreeMap::from([(0, "src/C.sol".to_string())]));
        let instructions = contract
            .info
            .instructions
            .iter()
            .map(|instruction| {
                json!({
                    "offset": instruction.offset,
                    "mnemonic": instruction.mnemonic(),
                    "context": instruction.context,
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            Value::Array(instructions),
            json!([
                {
                    "offset": 0,
                    "mnemonic": "PUSH1",
                    "context": {
                        "modifierDepth": 0,
                        "code": { "source": { "id": 0 }, "range": { "offset": 0, "length": 8 } },
                    },
                },
                {
                    "offset": 2,
                    "mnemonic": "JUMPDEST",
                    "context": {
                        "modifierDepth": 0,
                        "code": { "source": { "id": 0 }, "range": { "offset": 9, "length": 3 } },
                        "invoke": {},
                    },
                },
                { "offset": 3, "mnemonic": "STOP", "context": { "modifierDepth": 0, "return": {} } },
            ])
        );
    }

    #[test]
    fn selects_the_last_call_without_a_terminal() {
        let call = |label: Option<&str>, signature: Option<&str>| {
            let mut arena = CallTraceArena::default();
            let root = &mut arena.nodes_mut()[0].trace;
            root.address = Address::repeat_byte(0xa);
            root.data = Bytes::from_static(&[0x12, 0x34, 0x56, 0x78, 0x9a]);
            root.decoded = Some(Box::new(DecodedCallTrace {
                label: label.map(str::to_string),
                return_data: None,
                call_data: signature.map(|signature| DecodedCallData {
                    signature: signature.to_string(),
                    args: vec![],
                }),
            }));
            arena
        };
        let set_up = call(Some("CounterTest"), Some("setUp()"));
        let unlabeled = format!("{}::0x12345678", Address::repeat_byte(0xa));
        let test = call(None, None);
        assert_eq!(call_label(&set_up), "CounterTest::setUp()");
        assert_eq!(call_label(&test), unlabeled);

        let selected = select_call(vec![set_up, test]).unwrap();
        assert_eq!(call_label(&selected), unlabeled);
        assert!(select_call(Vec::new()).is_err());
    }

    #[test]
    fn writes_a_bundle_soldb_loads() {
        let (sources, arena, names) = executed_c();
        let dir = tempfile::tempdir().unwrap();

        write_bundle(dir.path(), arena, &names, &sources, &Default::default()).unwrap();

        let address = hex::encode_prefixed(Address::repeat_byte(0xc));
        let mapping = soldb_ethdebug::read_json_file(&dir.path().join("contracts.json")).unwrap();
        assert_eq!(
            mapping,
            json!({ "contracts": [{ "address": address, "name": "C", "debug_dir": address }] })
        );
        let program = soldb_ethdebug::load_debug_program_with_sources(
            &dir.path().join(&address),
            "C",
            SourceMapEnvironment::Runtime,
            &[],
        )
        .unwrap()
        .unwrap();
        assert!(program.legacy);
        assert!(program.missing_sources.is_empty());
        assert_eq!(program.source_contents, BTreeMap::from([(0, "contract C {}".to_string())]));
        let offsets = program.info.instructions.iter().map(|i| i.offset).collect::<Vec<_>>();
        assert_eq!(offsets, [0, 2, 3]);
        let trace = std::fs::read_to_string(dir.path().join("trace.json")).unwrap();
        let trace = serde_json::from_str::<TransactionTrace>(&trace).unwrap();
        assert_eq!(trace.steps.len(), 2);
    }
}
