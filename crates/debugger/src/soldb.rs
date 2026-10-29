//! [soldb](https://github.com/walnuthq/soldb) frontend.
//!
//! Converts a recorded call trace into a soldb transaction trace and the local artifacts' source
//! maps into soldb debug info, then runs a soldb session over them or writes them as a bundle that
//! soldb's own tools read.

use alloy_primitives::{Address, B256, Bytes, hex, keccak256, map::HashSet};
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
    CallTrace, CallTraceArena, CallTraceNode, CallTraceStep, TraceMemberOrder,
    debug::{
        ArtifactData, ContractSources, DebugRecording, RecordedFrame, SourceData, StepConverter,
        quantity,
    },
};
use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};
use serde_json::{Map, Value, json};
use soldb_core::{
    ContractCreation, EvmWord, ExecutionCall, StepRecord, StepTable, TraceArtifacts,
    TraceCapabilities, TransactionTrace,
};
use soldb_debugger::{CodeGenerator, ContractDebugInfo, SourceCache, StorageLayout};
use soldb_ethdebug::{EthdebugInfo, Instruction, SourceLocation, SourceMapContext, SourceMapJump};
use soldb_repl::{BreakpointTarget, DebuggerCommand, DebuggerState, Output, Renderer, Session};
use soldb_tui::Exit;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{self, IsTerminal, Write},
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// The frames that run a breakpoint location, as the steps they span and their depth.
type BreakpointFrames = Vec<(Range<usize>, u64)>;

/// The frames that run each breakpoint's code address and program counter.
type BreakpointSteps = BTreeMap<(Address, usize), BreakpointFrames>;

/// A soldb session over the recorded call traces.
#[derive(Debug)]
pub(crate) struct SoldbDebugger {
    trace: TransactionTrace,
    contracts: Vec<ContractDebugInfo>,
    /// The program counter of each `vm.breakpoint` location and the frames that run it.
    breakpoints: Vec<(u64, BreakpointFrames)>,
}

impl SoldbDebugger {
    /// Converts `arenas`, whose node contracts are identified by `contract_names`.
    ///
    /// The steps are the ones `recordings` hold, one per arena, or the arenas' own steps when
    /// `recordings` is empty.
    pub(crate) fn new(
        mut arenas: Vec<CallTraceArena>,
        recordings: Vec<DebugRecording>,
        contract_names: &[Vec<Option<String>>],
        sources: &ContractSources,
        known_contracts: &ContractsByArtifact,
        breakpoints: &Breakpoints,
    ) -> Result<Self> {
        let mut source_cache = SourceCache::default();
        let contracts = executed_programs(&arenas, contract_names, sources, known_contracts)
            .iter()
            .filter_map(|program| contract_debug_info(program, &mut source_cache))
            .collect();
        let (trace, breakpoints) = transaction_trace(&mut arenas, recordings, breakpoints)?;
        let breakpoints = breakpoints
            .into_iter()
            .filter(|(_, frames)| !frames.is_empty())
            .map(|((_, pc), frames)| (pc as u64, frames))
            .collect();
        Ok(Self { trace, contracts, breakpoints })
    }

    /// Runs the session.
    ///
    /// If stdin and stdout are terminals, the full-screen view opens first. Then commands are
    /// read from stdin, one per line, until `quit` or the end of the input.
    pub(crate) fn try_run(self) -> Result<()> {
        let mut state = DebuggerState::new();
        state.load_trace_with_debug_info(self.trace, self.contracts)?;
        // soldb breakpoints match a program counter in any contract, so each one is limited to
        // the own steps of the frames that run the `vm.breakpoint` location: the steps they span
        // at their depth, which leaves out their subcalls.
        for (pc, frames) in self.breakpoints {
            let condition = frames
                .iter()
                .map(|(steps, depth)| {
                    format!("step >= {} && step < {} && depth == {depth}", steps.start, steps.end)
                })
                .collect::<Vec<_>>()
                .join(" || ");
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

/// Converts `arenas` into a soldb trace: their steps in execution order, with each call and
/// creation recorded at the steps it spans.
///
/// The steps are the ones `recordings` hold, one per arena, or the arenas' own steps when
/// `recordings` is empty. Also returns the steps that run each `breakpoints` location. The steps
/// move out of `arenas`, so each one is dropped once converted.
fn transaction_trace(
    arenas: &mut [CallTraceArena],
    recordings: Vec<DebugRecording>,
    breakpoints: &Breakpoints,
) -> Result<(TransactionTrace, BreakpointSteps)> {
    let nodes = arenas.iter().flat_map(|arena| arena.nodes());
    let (steps, frames) =
        nodes.fold((0, 0), |(steps, frames), node| (steps + node.trace.steps.len(), frames + 1));
    let mut builder = TraceBuilder {
        calls: Vec::with_capacity(frames),
        creations: Vec::with_capacity(frames),
        breakpoints: breakpoints.values().map(|location| (*location, Vec::new())).collect(),
        ..Default::default()
    };
    if recordings.is_empty() {
        builder.steps = StepTable::with_capacity(steps);
        for arena in &mut *arenas {
            builder.visit(arena.nodes_mut(), 0, None);
        }
    } else {
        eyre::ensure!(recordings.len() == arenas.len(), "a debug trace has no recorded steps");
        let offsets;
        (builder.steps, offsets) = recorded_steps(&recordings)?;
        for ((arena, recording), offset) in arenas.iter_mut().zip(&recordings).zip(offsets) {
            let nodes = arena.nodes_mut();
            eyre::ensure!(
                recording.frames.len() == nodes.len()
                    && recording
                        .frames
                        .iter()
                        .zip(&*nodes)
                        .all(|(frame, node)| frame.depth == node.trace.depth),
                "the recorded steps do not match the call trace"
            );
            builder.visit_recorded(nodes, 0, None, &recording.frames, offset);
        }
    }
    let TraceBuilder { steps, calls, creations, breakpoints, .. } = builder;

    // The arenas run in order, such as the deployment, the setup, and the test, so the last one
    // is the call being debugged.
    let root = &arenas.last().ok_or_eyre("debug arena is empty")?.nodes()[0].trace;
    let create = root.kind.is_any_create();
    let trace = TransactionTrace {
        tx_hash: None,
        from_addr: hex::encode_prefixed(root.caller),
        to_addr: (!create).then(|| hex::encode_prefixed(root.address)),
        value: quantity(root.value),
        input_data: hex::encode_prefixed(&root.data),
        gas_used: arenas
            .iter()
            .fold(0u64, |gas, arena| gas.saturating_add(arena.nodes()[0].trace.gas_used)),
        output: hex::encode_prefixed(&root.output),
        success: arenas.iter().all(|arena| arena.nodes()[0].trace.success),
        error: arenas.iter().find_map(|arena| status_error(arena.nodes()[0].trace.status)),
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
            transient_storage: true,
            return_data: true,
            ..Default::default()
        },
        artifacts: TraceArtifacts { calls, creations, ..Default::default() },
        steps,
    };
    Ok((trace, breakpoints))
}

/// Collects the soldb steps, calls, and creations of a trace in execution order.
#[derive(Default)]
struct TraceBuilder {
    steps: StepTable,
    calls: Vec<ExecutionCall>,
    creations: Vec<ContractCreation>,
    /// Converts the return data and storage of the steps.
    converter: StepConverter,
    /// The frames that run each `vm.breakpoint` location, as code address and program counter.
    breakpoints: BreakpointSteps,
}

impl TraceBuilder {
    /// Records node `idx` and its subcalls. `parent` is the id of the enclosing call.
    fn visit(&mut self, nodes: &mut [CallTraceNode], idx: usize, parent: Option<usize>) {
        let node = &mut nodes[idx];
        let ordering = std::mem::take(&mut node.ordering);
        let children = std::mem::take(&mut node.children);
        let mut steps = std::mem::take(&mut node.trace.steps);
        let runs_steps = !steps.is_empty();
        let trace = &node.trace;
        let address = trace.address;
        let depth = trace.depth as u64 + 1;
        let entry_step = self.steps.len();
        let (creation, call) = self.open_frame(trace, parent, entry_step);

        // The ordering lists the steps in recording order.
        let mut next_step = 0;
        let mut hits = Vec::new();
        for order in ordering {
            match order {
                TraceMemberOrder::Step(_) => {
                    if next_step < steps.len() {
                        let (done, rest) = steps.split_at_mut(next_step + 1);
                        let step = &mut done[next_step];
                        if self.breakpoints.contains_key(&(address, step.pc))
                            && !hits.contains(&step.pc)
                        {
                            hits.push(step.pc);
                        }
                        self.push_step(step, rest.first(), depth);
                        // Free the converted snapshots now, not when the frame ends.
                        step.stack = None;
                        step.memory = None;
                        next_step += 1;
                    }
                }
                TraceMemberOrder::Call(child) => {
                    self.visit(nodes, children[child], call.or(parent));
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        self.close_frame(creation, call, self.steps.len(), runs_steps);
        self.record_hits(address, hits, entry_step..self.steps.len(), depth);
    }

    /// Records node `idx` and its subcalls, whose steps `frames` locate in the step table from
    /// `offset`. `parent` is the id of the enclosing call.
    fn visit_recorded(
        &mut self,
        nodes: &mut [CallTraceNode],
        idx: usize,
        parent: Option<usize>,
        frames: &[RecordedFrame],
        offset: usize,
    ) {
        let node = &mut nodes[idx];
        let ordering = std::mem::take(&mut node.ordering);
        let children = std::mem::take(&mut node.children);
        let address = node.trace.address;
        let frame = frames[idx];
        let (creation, call) = self.open_frame(&node.trace, parent, offset + frame.entry_step);

        // The frame's own steps are the ones between its subcalls.
        let mut next_step = offset + frame.entry_step;
        let mut runs_steps = false;
        let mut hits = Vec::new();
        for order in ordering {
            if let TraceMemberOrder::Call(child) = order {
                let child = children[child];
                let own = next_step..offset + frames[child].entry_step;
                runs_steps |= !own.is_empty();
                self.find_hits(address, own, &mut hits);
                self.visit_recorded(nodes, child, call.or(parent), frames, offset);
                next_step = offset + frames[child].exit_step;
            }
        }
        let own = next_step..offset + frame.exit_step;
        runs_steps |= !own.is_empty();
        self.find_hits(address, own, &mut hits);

        let steps = offset + frame.entry_step..offset + frame.exit_step;
        self.close_frame(creation, call, steps.end, runs_steps);
        self.record_hits(address, hits, steps, frame.depth as u64 + 1);
    }

    /// Adds the program counters of the `steps` that run a breakpoint location of `address` to
    /// `hits`.
    fn find_hits(&self, address: Address, steps: Range<usize>, hits: &mut Vec<usize>) {
        if self.breakpoints.range((address, 0)..=(address, usize::MAX)).next().is_none() {
            return;
        }
        for index in steps {
            let pc = self.steps.step(index).pc() as usize;
            if self.breakpoints.contains_key(&(address, pc)) && !hits.contains(&pc) {
                hits.push(pc);
            }
        }
    }

    /// Records that the frame of `address` that spans `steps` at `depth` runs the breakpoint
    /// locations at the program counters `hits`.
    fn record_hits(&mut self, address: Address, hits: Vec<usize>, steps: Range<usize>, depth: u64) {
        for pc in hits {
            if let Some(frames) = self.breakpoints.get_mut(&(address, pc)) {
                frames.push((steps.clone(), depth));
            }
        }
    }

    /// Records the call or creation `trace` that starts at `entry_step`, and returns its
    /// creation or call id.
    fn open_frame(
        &mut self,
        trace: &CallTrace,
        parent: Option<usize>,
        entry_step: usize,
    ) -> (Option<usize>, Option<usize>) {
        let depth = trace.depth as u64 + 1;
        let entry_step = Some(entry_step);
        let error = status_error(trace.status);
        // The code the frame runs, as `executed_programs` hashes it, so each frame maps through
        // its own program when several run at one address.
        let code_hash = trace.bytecode.as_ref().map(|code| keccak256(code).to_string());
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
                code_hash,
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
                code_hash,
            });
        }
        (creation, call)
    }

    /// Records that the creation or call that `open_frame` returned ends before `exit_step`, and
    /// whether it `runs_steps` of its own.
    ///
    /// SolDB finds the frame that runs a step by the step it was entered at. A frame that runs no
    /// step of its own but whose subcalls do, such as `vm.deployCode` around the creation it
    /// makes, would share its entry step with its first subcall and take the subcall's steps, so
    /// it records no entry step.
    fn close_frame(
        &mut self,
        creation: Option<usize>,
        call: Option<usize>,
        exit_step: usize,
        runs_steps: bool,
    ) {
        let (entry, exit) = if let Some(id) = creation {
            let creation = &mut self.creations[id];
            (&mut creation.entry_step, &mut creation.exit_step)
        } else if let Some(id) = call {
            let call = &mut self.calls[id];
            (&mut call.entry_step, &mut call.exit_step)
        } else {
            return;
        };
        if !runs_steps && entry.is_some_and(|entry| entry < exit_step) {
            *entry = None;
        }
        *exit = Some(exit_step);
    }

    /// Records `step`. `next` is the following step of the same frame, if any.
    fn push_step(&mut self, step: &CallTraceStep, next: Option<&CallTraceStep>, depth: u64) {
        let stack = step.stack.as_deref().unwrap_or_default();
        // The tracer shares one buffer between steps that leave memory unchanged, so this only
        // bumps a reference count.
        let memory = step.memory.as_ref().map(|memory| memory.as_bytes().0.clone());
        // The full stack snapshots do not record pushes, but an `SLOAD` or `TLOAD` leaves its
        // value on top of the next step's stack.
        let loaded = next.and_then(|next| next.stack.as_deref()?.last().copied());
        let had_value = step
            .storage_change
            .as_ref()
            .filter(|_| step.op == OpCode::SSTORE)
            .and_then(|change| change.had_value);
        let storage = self.converter.storage(step.op.get(), stack, loaded, had_value);
        // `None` would keep the previous step's buffer, so an empty one is spelled out too.
        let return_data = self.converter.return_data(&step.returndata);
        self.steps.append(
            StepRecord {
                pc: step.pc as u64,
                op: step.op.get(),
                gas: step.gas_remaining,
                gas_cost: step.gas_cost,
                depth,
                error: status_error(step.status),
                memory,
                storage: Some(storage.storage),
                storage_diff: storage.storage_diff,
                transient_storage: storage.transient_storage,
                return_data: Some(return_data),
            },
            stack.iter().map(|word| EvmWord(word.to_be_bytes())),
        );
    }
}

/// The step table that holds the steps of `recordings`, and where each recording's steps start in
/// it.
///
/// The recorders of one session append to one table in execution order, so it is taken as it is
/// when the recordings cover it in order. Otherwise their steps are copied into a new table.
fn recorded_steps(recordings: &[DebugRecording]) -> Result<(StepTable, Vec<usize>)> {
    let first = &recordings[0].sink;
    let mut offsets = Vec::with_capacity(recordings.len());
    let mut next = 0;
    for recording in recordings {
        if !recording.sink.same(first)
            || (!recording.steps.is_empty() && recording.steps.start != next)
        {
            offsets.clear();
            break;
        }
        offsets.push(next);
        next = recording.steps.end.max(next);
    }
    let mut shared = first.lock();
    let len = shared.as_ref().ok_or_eyre("the recorded steps are in use")?.len();
    if offsets.len() == recordings.len() && next == len {
        return Ok((shared.take().unwrap_or_default(), offsets));
    }
    drop(shared);

    offsets.clear();
    let mut table = StepTable::with_capacity(recordings.iter().map(|r| r.steps.len()).sum());
    for recording in recordings {
        offsets.push(table.len());
        let source = recording.sink.lock();
        let source = source.as_ref().ok_or_eyre("the recorded steps are in use")?;
        for index in recording.steps.clone() {
            let step = source.step(index);
            let stack = step.stack();
            table.append(
                StepRecord {
                    pc: step.pc(),
                    op: step.op(),
                    gas: step.gas(),
                    gas_cost: step.gas_cost(),
                    depth: step.depth(),
                    error: step.error().map(str::to_owned),
                    memory: step.memory(),
                    storage: Some(Arc::new(step.storage().clone())),
                    storage_diff: Some(step.storage_diff().clone()).filter(|diff| !diff.is_empty()),
                    transient_storage: Some(step.transient_storage().clone())
                        .filter(|slots| !slots.is_empty()),
                    return_data: Some(Arc::from(step.return_data().unwrap_or_default())),
                },
                (0..stack.len()).filter_map(|position| stack.get(position)),
            );
        }
    }
    Ok((table, offsets))
}

/// A program the trace executes and the local artifact that describes it.
struct ExecutedProgram<'a> {
    address: Address,
    name: &'a str,
    code: &'a Bytes,
    /// The hash of `code`, which tells apart programs run at one address.
    code_hash: B256,
    /// The hash of the deployed code: `code_hash` for deployed code, and for creation code the
    /// hash of the code it returned, so a contract's two programs share a bundle entry.
    deployed_hash: B256,
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

/// The programs `arenas` run that local artifacts describe, once per address and environment.
fn executed_programs<'a>(
    arenas: &'a [CallTraceArena],
    contract_names: &'a [Vec<Option<String>>],
    sources: &'a ContractSources,
    known_contracts: &ContractsByArtifact,
) -> Vec<ExecutedProgram<'a>> {
    let mut seen = HashSet::<(Address, bool, B256)>::default();
    let mut programs = Vec::new();
    for (node, name) in
        arenas.iter().zip(contract_names).flat_map(|(arena, names)| arena.nodes().iter().zip(names))
    {
        let trace = &node.trace;
        let init = trace.kind.is_any_create();
        if let Some(name) = name
            && let Some(code) = &trace.bytecode
            && let code_hash = keccak256(code)
            && seen.insert((trace.address, init, code_hash))
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
                code_hash,
                deployed_hash: if init { keccak256(&trace.output) } else { code_hash },
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
fn contract_debug_info(
    program: &ExecutedProgram<'_>,
    source_cache: &mut SourceCache,
) -> Option<ContractDebugInfo> {
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
            let mnemonic = op.map_or_else(
                || Cow::Owned(format!("UNKNOWN(0x{byte:02x})")),
                |op| Cow::Borrowed(op.as_str()),
            );
            instructions.push(Instruction::from_source_map(
                pc as u64,
                mnemonic,
                instruction_context(element, program.build_sources, &mut used_sources),
            ));
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
        ContractDebugInfo::with_source_cache(
            Some(&hex::encode_prefixed(program.address)),
            program.name,
            info,
            source_contents,
            source_cache,
        )
        .with_code_generator(code_generator)
        .with_code_hash(Some(&program.code_hash.to_string()))
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
    mut arenas: Vec<CallTraceArena>,
    recordings: Vec<DebugRecording>,
    contract_names: &[Vec<Option<String>>],
    sources: &ContractSources,
    known_contracts: &ContractsByArtifact,
) -> Result<()> {
    // The bundle copies the compiler's sources under their project paths, and they can differ
    // from the files on disk, so a directory with files, such as the project root, could have its
    // files replaced.
    if std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()) {
        eyre::bail!(
            "cannot write the SolDB bundle to {}: the directory is not empty",
            dir.display()
        );
    }
    // One entry per deployed code, so code replaced at an address keeps its own source maps.
    let mut contracts = BTreeMap::<(Address, B256), CombinedJson>::new();
    let mut written_sources = HashSet::<PathBuf>::default();
    for program in executed_programs(&arenas, contract_names, sources, known_contracts) {
        // soldb reads legacy source maps by instruction counter, which Vyper's are not.
        if !program.solc {
            continue;
        }
        let contract =
            contracts.entry((program.address, program.deployed_hash)).or_insert_with(|| {
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

    let mut programs_at = HashMap::<Address, usize>::new();
    for (address, _) in contracts.keys() {
        *programs_at.entry(*address).or_default() += 1;
    }
    let mut mapping = Vec::new();
    for ((address, code_hash), contract) in contracts {
        let debug_dir = if programs_at[&address] > 1 {
            format!("{}-{}", hex::encode_prefixed(address), hex::encode(&code_hash[..4]))
        } else {
            hex::encode_prefixed(address)
        };
        fs::create_dir_all(dir.join(&debug_dir))?;
        write_json_file(
            &dir.join(&debug_dir).join("combined.json"),
            &json!({
                "sourceList": contract.source_list,
                "contracts": { contract.key: contract.fields },
            }),
        )?;
        mapping.push(json!({
            "address": hex::encode_prefixed(address),
            "name": contract.name,
            "debug_dir": debug_dir,
            "code_hash": code_hash.to_string(),
        }));
    }
    write_json_file(&dir.join("contracts.json"), &json!({ "contracts": mapping }))?;
    let (trace, _) = transaction_trace(&mut arenas, recordings, &Breakpoints::default())?;
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
) -> SourceMapContext {
    // Compiler-generated code points at sources that are not in the build.
    let code = element.index().filter(|index| build_sources.contains_key(index)).map(|index| {
        used_sources.insert(index);
        SourceLocation {
            source_id: u64::from(index),
            offset: u64::from(element.offset()),
            length: u64::from(element.length()),
        }
    });
    SourceMapContext {
        code,
        jump: match element.jump() {
            Jump::In => SourceMapJump::Into,
            Jump::Out => SourceMapJump::Out,
            Jump::Regular => SourceMapJump::Regular,
        },
        modifier_depth: element.modifier_depth().into(),
    }
}

/// The error a failed step or call ended with.
fn status_error(status: Option<InstructionResult>) -> Option<String> {
    status.filter(|status| !status.is_ok()).map(|status| format!("{status:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use foundry_compilers::artifacts::sourcemap;
    use foundry_evm_traces::CallKind;
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
        let (trace, breakpoints) =
            transaction_trace(&mut [arena], Vec::new(), &breakpoints).unwrap();
        assert_eq!(
            breakpoints,
            BTreeMap::from([((proxy, 3), vec![(0..6, 1)]), ((implementation, 9), Vec::new())])
        );

        let steps = trace
            .steps
            .iter()
            .map(|step| (step.pc(), step.mnemonic().to_string(), step.depth()))
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
            |index: usize| serde_json::to_value(trace.steps.step(index).storage_diff()).unwrap();
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

        let arenas = [arena];
        let names = [names];
        let programs = executed_programs(&arenas, &names, &sources, &Default::default());
        let contract = contract_debug_info(&programs[0], &mut SourceCache::default()).unwrap();

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
                    "context": serde_json::to_value(instruction).unwrap()["context"],
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
    fn keeps_all_root_calls_and_breakpoints() {
        let (_, mut deployment, _) = executed_c();
        deployment.nodes_mut()[0].trace.kind = CallKind::Create;
        deployment.nodes_mut()[0].trace.gas_used = 10;
        deployment.nodes_mut()[0].trace.success = true;
        let (_, mut set_up, _) = executed_c();
        set_up.nodes_mut()[0].trace.gas_used = 20;
        set_up.nodes_mut()[0].trace.success = true;
        let (_, mut test, _) = executed_c();
        test.nodes_mut()[0].trace.gas_used = 30;
        test.nodes_mut()[0].trace.status = Some(InstructionResult::Revert);
        let breakpoints = Breakpoints::from_iter([('a', (Address::repeat_byte(0xc), 2))]);
        let (trace, hits) =
            transaction_trace(&mut [deployment, set_up, test], Vec::new(), &breakpoints).unwrap();
        soldb_debugger::check_root_calls(&trace).unwrap();
        assert_eq!(trace.gas_used, 60);
        // The header describes the test call, not the deployment.
        assert_eq!(trace.to_addr, Some(hex::encode_prefixed(Address::repeat_byte(0xc))));
        assert_eq!(trace.contract_address, None);
        assert!(!trace.success);
        assert_eq!(trace.error.as_deref(), Some("Revert"));
        assert_eq!(trace.steps.len(), 6);
        assert_eq!(trace.artifacts.creations[0].entry_step, Some(0));
        assert_eq!(trace.artifacts.creations[0].exit_step, Some(2));
        assert_eq!(trace.artifacts.calls[0].entry_step, Some(2));
        assert_eq!(trace.artifacts.calls[1].entry_step, Some(4));
        assert_eq!(trace.artifacts.calls[1].exit_step, Some(6));
        assert!(trace.artifacts.calls.iter().all(|call| call.parent_id.is_none()));
        assert_eq!(hits[&(Address::repeat_byte(0xc), 2)], [(0..2, 1), (2..4, 1), (4..6, 1)]);
        assert!(transaction_trace(&mut [], Vec::new(), &Breakpoints::default()).is_err());
    }

    #[test]
    fn records_transient_storage_and_return_data() {
        let mut builder = TraceBuilder::default();
        let store = step(0, OpCode::TSTORE, &[42, 7]);
        let load = step(1, OpCode::TLOAD, &[7]);
        let mut stop = step(2, OpCode::STOP, &[42]);
        stop.returndata = Bytes::from_static(&[0xab, 0xcd]);
        builder.push_step(&store, Some(&load), 1);
        builder.push_step(&load, Some(&stop), 1);
        builder.push_step(&stop, None, 1);
        builder.push_step(&stop, None, 1);
        builder.push_step(&store, None, 1);
        let expected = BTreeMap::from([("0x7".to_string(), "0x2a".to_string())]);
        assert_eq!(*builder.steps.step(0).transient_storage(), expected);
        assert_eq!(*builder.steps.step(1).transient_storage(), expected);
        assert!(builder.steps.step(2).transient_storage().is_empty());
        assert!(builder.steps.step(0).return_data().is_none());
        assert_eq!(builder.steps.step(2).return_data(), Some("abcd"));
        assert!(builder.steps.step(4).return_data().is_none());
        assert_eq!(
            builder.steps.step(2).return_data().unwrap().as_ptr(),
            builder.steps.step(3).return_data().unwrap().as_ptr(),
        );
    }

    #[test]
    fn writes_a_bundle_soldb_loads() {
        let (sources, arena, names) = executed_c();
        let dir = tempfile::tempdir().unwrap();

        write_bundle(dir.path(), vec![arena], Vec::new(), &[names], &sources, &Default::default())
            .unwrap();

        let address = hex::encode_prefixed(Address::repeat_byte(0xc));
        let mapping = soldb_ethdebug::read_json_file(&dir.path().join("contracts.json")).unwrap();
        let code_hash = keccak256([0x60, 0x80, 0x5b, 0x00]).to_string();
        assert_eq!(
            mapping,
            json!({ "contracts": [{
                "address": address,
                "name": "C",
                "debug_dir": address,
                "code_hash": code_hash,
            }] })
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

        // A second bundle would replace the files of the first, as it would a project's sources.
        let (sources, arena, names) = executed_c();
        let error = write_bundle(
            dir.path(),
            vec![arena],
            Vec::new(),
            &[names],
            &sources,
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.to_string().ends_with("the directory is not empty"));
    }
}
