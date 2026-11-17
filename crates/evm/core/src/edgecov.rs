//! Compiler-instrumented edge coverage, comparison logging, and source coverage through the
//! `EDGECOV`, `CMPLOG`, and `COVERAGE` opcodes.
//!
//! Coverage builds of a compiler emit `PUSHn <id>; EDGECOV` on every control-flow edge and
//! `PUSHn <site << 8 | cmp opcode>; CMPLOG` right before every `EQ`, `LT`, `GT`, `SLT` and `SGT`.
//! Source coverage builds, such as Solar's `-Zinstrument-coverage`, emit `COVERAGE; PUSHn
//! <counter>` where a function, statement, or branch starts. The opcodes are otherwise undefined.
//! Foundry's EVMs carry them in their opcode tables, but they fail as undefined opcodes unless an
//! [`EdgeCovGuard`] is alive on the thread.
//!
//! While it is, `EDGECOV` pops the edge ID and bumps its AFL NeverZero hit counter, and `CMPLOG`
//! pops its tag and records the two words below it, the operands of the comparison that follows.
//! Both refund the 3 gas of the `PUSH` the compiler emits before them, so a probe costs no gas
//! once it ran. Hits and operands are kept per thread in dense tables indexed by ID, and taken
//! once the call ends.
//!
//! `COVERAGE` skips the push after it, so a probe costs no gas or stack at any point, and JUMPDEST
//! analysis still sees a whole push. It reads the push itself. A plain probe, with a counter ID
//! below `2^31`, counts a hit of that counter in the table of the executing code, and an MC/DC
//! probe, with bit 31 set, drives the decision stack that records which test vectors of a
//! decision ran. The tables are per code object, keyed by code hash, and taken with
//! [`take_coverage`] once the call ends. With edge recording on, plain probes also feed the edge
//! hits as AFL-style edges, the hash of the previous probe and this one in the same call frame, and
//! code that ran a probe no longer records its `EDGECOV` hits.

use alloy_primitives::{
    Address, B256, Bytes, U256,
    map::{
        B256HashMap,
        rustc_hash::{FxHashMap, FxHashSet},
    },
};
use evm2::{
    EvmConfig, EvmTypesHost, ExecutionConfig, OpcodeConfig, SpecId, Version,
    bytecode::Bytecode,
    interpreter::{InstrStop, InterpreterState, op},
};
use evm2_macros::instruction;
use std::{cell::UnsafeCell, marker::PhantomData};

/// The `EDGECOV` opcode byte.
const EDGECOV: u8 = 0xC0;

/// The `CMPLOG` opcode byte.
const CMPLOG: u8 = 0xC1;

/// The `COVERAGE` opcode byte.
pub const COVERAGE: u8 = 0xC2;

/// Set in the value of an MC/DC probe, whose other bits hold the operation, decision, and
/// increment.
const MCDC_FLAG: u32 = 1 << 31;

/// The mask of an edge ID that two probes hash to, as large as the dense edge tables allow.
const EDGE_MASK: u32 = (1 << 16) - 1;

/// The most code hashes the cache of which code ran probes holds before it starts over.
const MAX_PROBE_CACHE: usize = 1 << 16;

/// Gas of the `PUSHn` that precedes every `EDGECOV` or `CMPLOG`, refunded by the instruction.
const PUSH_GAS: u64 = 3;

/// Largest edge or comparison site ID that is recorded. IDs are dense from 1, so anything above
/// this is malformed instrumentation and is ignored instead of growing the tables without bound.
const MAX_EDGE_ID: u32 = 1 << 24;

/// Maximum number of unique comparison sites tracked per call for CmpLog-style feedback.
pub const MAX_CMP_LOG_SITES: usize = 1024;

/// Maximum number of comparison operand pairs tracked per site. This matches the downstream loop
/// detection threshold so a hot loop can be classified without filling the whole log.
pub const MAX_CMP_OBSERVATIONS_PER_SITE: u8 = 8;

/// A comparison operand pair captured during execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CmpOperands {
    /// First operand of the comparison.
    pub op1: U256,
    /// Second operand of the comparison.
    pub op2: U256,
    /// Program counter where the comparison occurred, or its site ID for a `CMPLOG` probe.
    pub pc: usize,
    /// Contract address where the comparison occurred.
    pub address: Address,
    /// EVM opcode that performed the comparison.
    pub opcode: u8,
}

/// One instrumented edge hit during a call: its ID and AFL-style hit counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstrumentedEdgeHit {
    /// Compiler-assigned edge ID.
    pub id: u32,
    /// Hit counter; never zero once the edge was hit (it saturates back to 1 on overflow).
    pub count: u8,
}

/// Hit counters of one call, indexed by edge ID.
#[derive(Debug)]
struct EdgeHits {
    counts: Vec<u8>,
    /// IDs hit since the last reset, in first-hit order.
    touched: Vec<u32>,
}

impl EdgeHits {
    fn record(&mut self, id: u32) {
        let index = id as usize;
        if index >= self.counts.len() {
            self.counts.resize((index + 1).next_power_of_two(), 0);
        }
        let count = &mut self.counts[index];
        if *count == 0 {
            self.touched.push(id);
        }
        // AFL NeverZero: a counter that wraps stays non-zero.
        *count = count.wrapping_add(1).max(1);
    }

    fn take(&mut self) -> Vec<InstrumentedEdgeHit> {
        let Self { counts, touched } = self;
        touched
            .drain(..)
            .map(|id| InstrumentedEdgeHit { id, count: std::mem::take(&mut counts[id as usize]) })
            .collect()
    }
}

/// Comparison operands of one call, with the cmp-log inspector's caps counted per site ID in a
/// dense table. Unlike the inspector, copies of a contract at several addresses share their caps.
#[derive(Debug)]
struct SiteCmpLog {
    log: Vec<CmpOperands>,
    /// Observations per site, indexed by site ID.
    counts: Vec<u8>,
    /// Sites observed since the last reset.
    touched: Vec<u32>,
}

impl SiteCmpLog {
    fn record(&mut self, site: u32, cmp: impl FnOnce() -> CmpOperands) {
        let index = site as usize;
        if index >= self.counts.len() {
            self.counts.resize((index + 1).next_power_of_two(), 0);
        }
        let count = &mut self.counts[index];
        if *count >= MAX_CMP_OBSERVATIONS_PER_SITE {
            return;
        }
        if *count == 0 {
            if self.touched.len() >= MAX_CMP_LOG_SITES {
                return;
            }
            self.touched.push(site);
        }
        *count += 1;
        self.log.push(cmp());
    }

    fn take(&mut self) -> Vec<CmpOperands> {
        for site in self.touched.drain(..) {
            self.counts[site as usize] = 0;
        }
        std::mem::take(&mut self.log)
    }
}

/// This thread's instrumentation state.
#[derive(Debug)]
struct State {
    /// How many guards are alive on this thread. The opcodes execute while it is nonzero. A
    /// counter rather than a flag keeps them enabled when a nested guard, such as one a cheatcode
    /// or rayon work-stealing starts during a transaction, is dropped before the outer one.
    depth: u32,
    /// Whether `EDGECOV` records hits.
    edges: bool,
    /// Whether `CMPLOG` records operands.
    cmp_log: bool,
    hits: EdgeHits,
    cmps: SiteCmpLog,
    coverage: CoverageState,
}

thread_local! {
    // An `UnsafeCell` rather than a `RefCell` keeps the probes free of borrow-flag updates.
    static STATE: UnsafeCell<State> = const {
        UnsafeCell::new(State {
            depth: 0,
            edges: false,
            cmp_log: false,
            hits: EdgeHits { counts: Vec::new(), touched: Vec::new() },
            cmps: SiteCmpLog { log: Vec::new(), counts: Vec::new(), touched: Vec::new() },
            coverage: CoverageState::new(),
        })
    };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: the reference does not escape `f`, and nothing called from `f` re-enters this
    // function, so it is the only live reference to this thread's state.
    STATE.with(|state| f(unsafe { &mut *state.get() }))
}

/// Executes the instrumentation opcodes on this thread while alive, recording edge hits when
/// `edges` is set, comparison operands when `cmp_log` is set, and the hits of `COVERAGE` probes
/// per code object when `counting` is set.
#[derive(Debug)]
pub struct EdgeCovGuard(());

impl EdgeCovGuard {
    /// Enables the opcodes, and on the outermost guard clears the hits and operands of the
    /// previous call. A nested guard keeps the outer guard's recording settings and tables, so it
    /// does not reset another transaction's hits.
    pub fn new(edges: bool, cmp_log: bool, counting: bool) -> Self {
        with_state(|state| {
            if state.depth == 0 {
                state.edges = edges;
                state.cmp_log = cmp_log;
                state.hits.take();
                state.cmps.take();
                state.coverage.take();
                state.coverage.counting = counting;
            }
            state.depth += 1;
        });
        Self(())
    }
}

impl Drop for EdgeCovGuard {
    fn drop(&mut self) {
        with_state(|state| state.depth = state.depth.saturating_sub(1));
    }
}

/// Starts a call frame at `depth`: the first probe in it has no previous probe. Call it where a
/// frame starts, so that frames that the interpreter allocates alike stay apart.
#[inline]
pub fn enter_frame(depth: u16) {
    with_state(|state| {
        if state.depth != 0 && state.edges {
            state.coverage.enter_frame(depth);
        }
    });
}

/// Takes this thread's edge hits in first-hit order.
pub fn take_edges() -> Vec<InstrumentedEdgeHit> {
    with_state(|state| state.hits.take())
}

/// Takes this thread's comparison operands.
pub fn take_cmp_log() -> Vec<CmpOperands> {
    with_state(|state| state.cmps.take())
}

/// Takes this thread's `COVERAGE` hits of the code objects that ran, by code hash.
pub fn take_coverage() -> Vec<(B256, CodeCoverage)> {
    with_state(|state| state.coverage.take())
}

/// Spreads a probe ID over the bits of a word, so that sequential IDs do not share edges.
const fn mix(probe: u32) -> u32 {
    probe.wrapping_mul(0x9E37_79B1).rotate_left(16)
}

/// Converts a probe's ID word, ignoring the reserved ID 0 and IDs above [`MAX_EDGE_ID`].
fn probe_id(word: U256) -> Option<u32> {
    u32::try_from(word).ok().filter(|id| (1..=MAX_EDGE_ID).contains(id))
}

/// The `EDGECOV` instruction: pops the edge ID, records the hit and refunds the `PUSH` gas. Code
/// that ran a `COVERAGE` probe feeds the edge hits through its probes instead.
#[instruction(dynamic_gas, no_stack_preamble)]
fn edgecov(cx: _) -> Result {
    with_state(|state| {
        if state.depth == 0 {
            return Err(InstrStop::OpcodeNotFound);
        }
        let id = stack.pop()?;
        cx.gas.erase_cost(PUSH_GAS);
        if state.edges
            && let Some(id) = probe_id(id)
            && !(state.coverage.any_probe_seen && state.coverage.executing(&*cx.state).has_probes)
        {
            state.hits.record(id);
        }
        Ok(())
    })
}

/// The `CMPLOG` instruction: pops `site << 8 | cmp opcode`, records the two words below it as the
/// operands of that comparison, with the site ID as the program counter, and refunds the `PUSH`
/// gas.
#[instruction(dynamic_gas, no_stack_preamble)]
fn cmplog(cx: _) -> Result {
    with_state(|state| {
        if state.depth == 0 {
            return Err(InstrStop::OpcodeNotFound);
        }
        let tag = stack.pop()?;
        let [op1, op2] = stack.peekn::<2>().ok_or(InstrStop::StackUnderflow)?;
        cx.gas.erase_cost(PUSH_GAS);
        let opcode = tag.byte(0);
        if state.cmp_log
            && matches!(opcode, op::EQ | op::LT | op::GT | op::SLT | op::SGT)
            && let Some(site) = probe_id(tag >> 8)
        {
            let address = cx.state.message().destination;
            state.cmps.record(site, || CmpOperands {
                op1,
                op2,
                pc: site as usize,
                address,
                opcode,
            });
        }
        Ok(())
    })
}

/// The `COVERAGE` instruction: reads the `PUSHn <value>` after it, counts the probe, and skips the
/// push.
#[instruction(no_stack_preamble)]
fn coverage(cx: _) -> Result {
    with_state(|state| {
        if state.depth == 0 {
            return Err(InstrStop::OpcodeNotFound);
        }
        // SAFETY: Legacy bytecode is padded with 33 zero bytes, which covers the byte after the
        // last opcode and the immediate of a push that starts in the code.
        let push = unsafe { cx.pc.read_bytes_offset_unchecked(1, 1)[0] };
        if !(op::PUSH1..=op::PUSH32).contains(&push) {
            return Err(InstrStop::InvalidImmediateEncoding);
        }
        let len = usize::from(push - op::PUSH0);
        if (state.coverage.counting || state.edges)
            // The values of valid probes fit in four bytes.
            && len <= 4
        {
            // SAFETY: As above, the immediate is in the padded bytecode.
            let immediate = unsafe { cx.pc.read_bytes_offset_unchecked(2, len) };
            let value = immediate.iter().fold(0u32, |value, &byte| value << 8 | u32::from(byte));
            state.probe(&*cx.state, value);
        }
        // The dispatch then steps over the `COVERAGE` byte.
        unsafe { cx.pc.advance_unchecked(len + 1) };
        Ok(())
    })
}

impl State {
    /// Handles a `COVERAGE` probe with value `value` in the executing code.
    #[inline]
    fn probe<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>, value: u32) {
        self.coverage.note_probe(state);
        if value & MCDC_FLAG != 0 {
            if self.coverage.counting {
                self.coverage.mcdc(state, value);
            }
            return;
        }
        if self.coverage.counting {
            self.coverage.hit(state, value);
        }
        if self.edges && (1..=MAX_EDGE_ID).contains(&value) {
            // Like AFL, an edge is a pair of probes, so the first time a path reaches a probe by
            // another probe is new coverage, even if the probe itself ran before.
            let edge = self.coverage.edge(state, value);
            self.hits.record(edge);
        }
    }
}

/// The hits of one code object.
#[derive(Debug)]
struct CodeHits {
    /// The code. It keeps the allocation alive, so that its address identifies it.
    _code: Bytecode,
    bytecode: Bytes,
    counters: FxHashMap<u32, u32>,
    mcdc: FxHashSet<(u32, u32)>,
}

/// The counter hits and executed MC/DC test vectors of one code object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeCoverage {
    /// The code.
    pub bytecode: Bytes,
    /// The hits of each counter that ran.
    pub counters: Vec<(u32, u32)>,
    /// The executed test vectors as `(decision ID, test vector index)`.
    pub mcdc: Vec<(u32, u32)>,
}

/// The code a probe ran in last.
#[derive(Clone, Copy, Debug)]
struct Executing {
    /// The address of the code's bytes.
    ptr: *const u8,
    hash: B256,
    /// Whether the code ran a `COVERAGE` probe.
    has_probes: bool,
    /// The index of the code's table, once it has one.
    table: Option<usize>,
}

/// The edge state of a call frame: the previous probe in it.
#[derive(Clone, Copy, Debug)]
struct FrameEdges {
    depth: u16,
    prev: u32,
}

/// An MC/DC decision in progress.
#[derive(Clone, Copy, Debug)]
struct McdcEntry {
    depth: u16,
    decision: u32,
    index: u32,
}

/// This thread's `COVERAGE` state.
#[derive(Debug)]
struct CoverageState {
    /// Whether probes count hits per code object.
    counting: bool,
    executing: Executing,
    /// The code `executing` points into, kept alive so that no other code takes its address.
    pinned: Bytecode,
    tables: Vec<CodeHits>,
    maps: Option<Box<CodeMaps>>,
    /// The decisions in progress, innermost last.
    mcdc: Vec<McdcEntry>,
    /// The call frames that started since the last call, innermost last.
    frames: Vec<FrameEdges>,
    /// Whether any code ran a probe on this thread. Until one does, `EDGECOV` need not look up
    /// the executing code.
    any_probe_seen: bool,
}

/// Lookups by code hash.
#[derive(Debug, Default)]
struct CodeMaps {
    /// The index of each code object's table.
    table_of: B256HashMap<usize>,
    /// Whether the code with a hash ran a probe. It outlives the calls, so that code that ran one
    /// in an earlier call has no `EDGECOV` hits from its first instruction.
    has_probes: B256HashMap<bool>,
}

impl CoverageState {
    const fn new() -> Self {
        Self {
            counting: false,
            executing: Executing {
                ptr: std::ptr::null(),
                hash: B256::ZERO,
                has_probes: false,
                table: None,
            },
            pinned: Bytecode::new(),
            tables: Vec::new(),
            maps: None,
            mcdc: Vec::new(),
            frames: Vec::new(),
            any_probe_seen: false,
        }
    }

    /// Returns the code that runs in `state`, switching to it when it changed.
    #[inline]
    fn executing<T: EvmTypesHost>(
        &mut self,
        state: &InterpreterState<'_, '_, T>,
    ) -> &mut Executing {
        if self.executing.ptr != state.bytecode().as_ptr() {
            self.switch(state);
        }
        &mut self.executing
    }

    #[cold]
    #[inline(never)]
    fn switch<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>) {
        let code = &state.message().code;
        let hash = code.hash_slow();
        let maps = self.maps.get_or_insert_default();
        if maps.has_probes.len() >= MAX_PROBE_CACHE {
            maps.has_probes.clear();
        }
        let has_probes = maps.has_probes.get(&hash).copied().unwrap_or(false);
        self.executing = Executing {
            ptr: state.bytecode().as_ptr(),
            hash,
            has_probes,
            table: maps.table_of.get(&hash).copied(),
        };
        self.pinned = code.clone();
    }

    /// Records that the executing code ran a probe, which is what tells code with probes from code
    /// that has a `COVERAGE` byte in its data.
    #[inline]
    fn note_probe<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>) {
        self.any_probe_seen = true;
        let executing = self.executing(state);
        if !executing.has_probes {
            executing.has_probes = true;
            let hash = executing.hash;
            self.maps.get_or_insert_default().has_probes.insert(hash, true);
        }
    }

    /// Starts the call frame at `depth`, which has no previous probe. The frames at that depth
    /// and below it that ran before ended.
    fn enter_frame(&mut self, depth: u16) {
        while self.frames.last().is_some_and(|frame| frame.depth >= depth) {
            self.frames.pop();
        }
        self.frames.push(FrameEdges { depth, prev: 0 });
    }

    /// The AFL-style edge ID of a probe: the mixed probe and the previous one in the same call
    /// frame, hashed together. A call frame starts without a previous probe.
    ///
    /// Compilers number probes in sequence, so the IDs are mixed first, or the edges of one probe
    /// would differ in a few low bits and collide.
    fn edge<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>, probe: u32) -> u32 {
        let depth = state.message().depth;
        // The frames deeper than this one returned.
        while self.frames.last().is_some_and(|frame| frame.depth > depth) {
            self.frames.pop();
        }
        // A frame that no hook announced starts at its first probe.
        if self.frames.last().is_none_or(|frame| frame.depth != depth) {
            self.frames.push(FrameEdges { depth, prev: 0 });
        }
        let frame = self.frames.last_mut().expect("the frame was just pushed");
        let probe = mix(probe);
        let edge = (probe ^ frame.prev) & EDGE_MASK;
        frame.prev = probe >> 1;
        // The ID 0 is reserved.
        edge.max(1)
    }

    /// The table of the executing code, created on first use.
    #[inline]
    fn table<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>) -> &mut CodeHits {
        let executing = self.executing(state);
        let index = match executing.table {
            Some(index) => index,
            None => self.new_table(),
        };
        &mut self.tables[index]
    }

    #[cold]
    #[inline(never)]
    fn new_table(&mut self) -> usize {
        let index = self.tables.len();
        self.tables.push(CodeHits {
            _code: self.pinned.clone(),
            bytecode: self.pinned.original_bytes(),
            counters: FxHashMap::default(),
            mcdc: FxHashSet::default(),
        });
        self.maps.get_or_insert_default().table_of.insert(self.executing.hash, index);
        self.executing.table = Some(index);
        index
    }

    /// Counts a hit of a counter in the executing code.
    #[inline]
    fn hit<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>, counter: u32) {
        let hits = self.table(state).counters.entry(counter).or_default();
        *hits = hits.saturating_add(1);
    }

    /// Handles an MC/DC probe: bits 30 and 29 hold the operation, bits 28 to 8 the decision, and
    /// bits 7 to 0 the increment of an `add`.
    fn mcdc<T: EvmTypesHost>(&mut self, state: &InterpreterState<'_, '_, T>, value: u32) {
        let depth = state.message().depth;
        // The frames of the entries deeper than this one exited or reverted.
        while self.mcdc.last().is_some_and(|entry| entry.depth > depth) {
            self.mcdc.pop();
        }
        let decision = value >> 8 & 0x1f_ffff;
        match value >> 29 & 3 {
            // Begin.
            0 => self.mcdc.push(McdcEntry { depth, decision, index: 0 }),
            // Add.
            1 => {
                if let Some(entry) = self.mcdc.iter_mut().rev().find(|e| e.decision == decision) {
                    entry.index = entry.index.saturating_add(value & 0xff);
                }
            }
            // End: the decision completed, so its test vector counts. An unfinished decision
            // leaves nothing.
            2 => {
                if let Some(position) = self.mcdc.iter().rposition(|e| e.decision == decision) {
                    let entry = self.mcdc[position];
                    self.mcdc.truncate(position);
                    self.table(state).mcdc.insert((decision, entry.index));
                }
            }
            _ => {}
        }
    }

    /// Takes the hits of the code objects, and forgets the code that ran.
    fn take(&mut self) -> Vec<(B256, CodeCoverage)> {
        let hashes = self
            .maps
            .as_mut()
            .map(|maps| {
                maps.table_of.drain().map(|(hash, index)| (index, hash)).collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut tables = std::mem::take(&mut self.tables).into_iter().map(Some).collect::<Vec<_>>();
        let hits = hashes
            .into_iter()
            .filter_map(|(index, hash)| {
                let table = tables[index].take()?;
                let hits = CodeCoverage {
                    bytecode: table.bytecode,
                    counters: table.counters.into_iter().collect(),
                    mcdc: table.mcdc.into_iter().collect(),
                };
                Some((hash, hits))
            })
            .collect();
        self.executing =
            Executing { ptr: std::ptr::null(), hash: B256::ZERO, has_probes: false, table: None };
        self.pinned = Bytecode::new();
        self.mcdc.clear();
        self.frames.clear();
        hits
    }
}

/// Adds the `EDGECOV`, `CMPLOG`, and `COVERAGE` instructions to `config`.
pub(crate) const fn with_instrumentation<T: EvmTypesHost>(
    mut config: OpcodeConfig<T>,
) -> OpcodeConfig<T> {
    config.set_instruction::<edgecov<T>>(EDGECOV, 0);
    config.set_instruction::<cmplog<T>>(CMPLOG, 0);
    config.set_instruction::<coverage<T>>(COVERAGE, 0);
    config
}

/// The base opcode table of `BASE_SPEC_ID` with the instrumentation opcodes.
struct InstrumentedConfig<T, const BASE_SPEC_ID: u32>(PhantomData<fn() -> T>);

impl<T: EvmTypesHost, const BASE_SPEC_ID: u32> EvmConfig<T>
    for InstrumentedConfig<T, BASE_SPEC_ID>
{
    const BASE_SPEC_ID: SpecId = SpecId::try_from_u32(BASE_SPEC_ID).expect("invalid base spec id");
    const OPCODE_CONFIG: &'static OpcodeConfig<T> =
        &with_instrumentation(OpcodeConfig::base::<Self>());
}

/// The execution config of a network that runs the base opcode table, with the instrumentation
/// opcodes.
pub(crate) fn execution_config<T: EvmTypesHost>(
    spec: T::SpecId,
    version: Version,
) -> ExecutionConfig<T> {
    evm2::spec_to_generic!(spec.into(), |BASE_SPEC_ID| ExecutionConfig::for_config::<
        InstrumentedConfig<T, BASE_SPEC_ID>,
    >()
    .with_version(version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::{EthEvmFactory, FoundryEvmFactory, SpecOf};
    use evm2::OpcodeChange;
    use tempo_evm::TempoEvmFactory;

    fn hits(ids: &[u32]) -> Vec<InstrumentedEdgeHit> {
        let _guard = EdgeCovGuard::new(true, false, false);
        with_state(|state| {
            for &id in ids {
                if let Some(id) = probe_id(U256::from(id)) {
                    state.hits.record(id);
                }
            }
        });
        take_edges()
    }

    #[test]
    fn counter_never_wraps_to_zero() {
        assert_eq!(hits(&[5; 256]), [InstrumentedEdgeHit { id: 5, count: 1 }]);
        assert_eq!(hits(&[5; 255]), [InstrumentedEdgeHit { id: 5, count: 255 }]);
    }

    #[test]
    fn ignores_reserved_and_out_of_range_ids() {
        assert!(hits(&[0, MAX_EDGE_ID + 1]).is_empty());
        assert_eq!(probe_id(U256::MAX), None);
    }

    /// Asserts that `F`'s opcode table is its network's default table plus the probes.
    fn assert_adds_only_probes<F: FoundryEvmFactory>() {
        let spec = SpecOf::<F>::default();
        let version = F::default().version(spec, 1);
        let ours = F::default().execution_config(spec, version).opcode_view();
        let default =
            ExecutionConfig::<F::Types>::for_spec_and_version(spec, version).opcode_view();
        for opcode in 0..=u8::MAX {
            let expected = match opcode {
                EDGECOV | CMPLOG | COVERAGE => OpcodeChange::Instruction,
                _ => default.change(opcode),
            };
            assert_eq!(ours.change(opcode), expected, "opcode {opcode:#04x}");
            assert_eq!(ours.static_gas(opcode), default.static_gas(opcode), "opcode {opcode:#04x}");
        }
    }

    #[test]
    fn networks_add_only_probes_to_their_opcode_tables() {
        assert_adds_only_probes::<EthEvmFactory>();
        assert_adds_only_probes::<TempoEvmFactory>();
        #[cfg(feature = "optimism")]
        assert_adds_only_probes::<op_evm2::OpEvmFactory>();
        #[cfg(feature = "base")]
        assert_adds_only_probes::<base_evm2::BaseEvmFactory>();
    }
}
