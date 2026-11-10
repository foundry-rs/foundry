//! Compiler-instrumented edge coverage through the `EDGECOV` opcode, and the instruction hooks
//! that replace per-opcode inspector work while it is on.
//!
//! Coverage builds of a compiler can emit `PUSHn <id>; EDGECOV` on every control-flow edge,
//! where `EDGECOV` is the otherwise undefined opcode `0xC0`. When the fuzzer runs with
//! `instrumented_edges`, the executor's EVM gets an instruction for that opcode which pops the
//! edge ID and bumps its hit counter in a thread-local dense map, instead of deriving edges from
//! every `JUMP`/`JUMPI` with an inspector.
//!
//! The instruction charges no gas and refunds the 3 gas of the preceding `PUSH`, so instrumentation
//! only changes code size. Hits are recorded per thread: the executor resets the map before a
//! call and drains it once the call finishes.
//!
//! In the same mode, the other per-opcode signals of a fuzz campaign come from wrapped
//! instructions instead of `Inspector::step`, so the inspector needs no opcode dispatch:
//!
//! - comparison operands (cmp-log) from the compiler-instrumented [`CMPLOG`] opcode, which coverage
//!   builds emit as `PUSHn <site << 8 | cmp opcode>; CMPLOG` right before each `EQ`, `LT`, `GT`,
//!   `SLT` and `SGT`, with the same per-site caps as the cmp-log inspector, kept in a dense table
//!   indexed by site instead of a hash map;
//! - the caller's stack at each `CALL`/`CREATE`-family instruction that opens a frame, which the
//!   fuzzer's dictionary collector reads once the frame returns ([`take_pending_frame_stack`]);
//! - 64-byte `KECCAK256` preimages and `SSTORE` slots, from which the fuzzer recovers mapping slots
//!   ([`take_mapping_events`]).
//!
//! Every wrapper records and then runs revm's default instruction for its opcode, which all
//! networks that install these hooks (Ethereum, Optimism, Base, Tempo) use for these opcodes.
//! With no inspector left that needs `step`, Ethereum frames then run on the interpreter's plain
//! loop (see [`InspectorExt::run_frame_plain`](crate::InspectorExt::run_frame_plain)).

use alloy_primitives::{Address, B256, U256};
use revm::{
    bytecode::opcode,
    handler::instructions::EthInstructions,
    interpreter::{
        Host, Instruction, InstructionContext, InstructionExecResult, InstructionResult,
        InterpreterAction, instruction_table,
        interpreter::EthInterpreter,
        interpreter_types::{InputsTr, InterpreterTypes, StackTr},
    },
};
use std::cell::{RefCell, UnsafeCell};

/// The `EDGECOV` opcode byte.
pub const EDGECOV: u8 = 0xC0;

/// The `CMPLOG` opcode byte: pops `site << 8 | cmp opcode` and records the two words below it,
/// the operands of the comparison that follows, leaving them on the stack.
pub const CMPLOG: u8 = 0xC1;

/// Gas of the `PUSHn` that precedes every `EDGECOV` or `CMPLOG`, refunded by the instruction.
const PUSH_GAS: u64 = 3;

/// Largest edge ID that is recorded. IDs are dense from 1, so anything above this is malformed
/// instrumentation and is ignored instead of growing the map without bound.
pub const MAX_EDGE_ID: u32 = 1 << 24;

/// Maximum number of unique comparison sites tracked per call for CmpLog-style feedback.
pub const MAX_CMP_LOG_SITES: usize = 1024;

/// Maximum number of comparison operand pairs tracked per site. This matches the downstream loop
/// detection threshold so a hot loop can be classified without filling the whole log.
pub const MAX_CMP_OBSERVATIONS_PER_SITE: u8 = 8;

/// Which instruction hooks an EVM gets, as its inspector reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EvmHooks {
    /// Execute the compiler-instrumented [`EDGECOV`] opcode instead of treating it as undefined.
    pub edgecov: bool,
    /// Execute the compiler-instrumented [`CMPLOG`] opcode, recording comparison operands into
    /// this thread's cmp-log ([`take_cmp_log`]), instead of treating it as undefined.
    pub cmp_log: bool,
    /// Record the caller's stack when a `CALL`/`CREATE`-family instruction opens a frame
    /// ([`take_pending_frame_stack`]).
    pub frame_stacks: bool,
    /// Record 64-byte `KECCAK256` preimages and `SSTORE` slots ([`take_mapping_events`]).
    pub mapping_slots: bool,
}

impl EvmHooks {
    /// Whether any hook is on.
    pub const fn any(self) -> bool {
        self.edgecov || self.cmp_log || self.frame_stacks || self.mapping_slots
    }
}

/// One instrumented edge hit during a call: its ID and AFL-style hit counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentedEdgeHit {
    /// Compiler-assigned edge ID.
    pub id: u32,
    /// Hit counter; never zero once the edge was hit (it saturates back to 1 on overflow).
    pub count: u8,
}

/// A comparison operand pair captured during execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CmpOperands {
    /// First operand of the comparison.
    pub op1: U256,
    /// Second operand of the comparison.
    pub op2: U256,
    /// Program counter where the comparison occurred.
    pub pc: usize,
    /// Contract address where the comparison occurred.
    pub address: Address,
    /// EVM opcode that performed the comparison.
    pub opcode: u8,
}

/// Per-thread hit map for the current call.
#[derive(Debug, Default)]
struct EdgeHits {
    /// Hit counters indexed by edge ID, grown on demand.
    counts: Vec<u8>,
    /// IDs hit since the last reset, in first-hit order.
    touched: Vec<u32>,
}

impl EdgeHits {
    const fn new() -> Self {
        Self { counts: Vec::new(), touched: Vec::new() }
    }

    #[inline(always)]
    fn record(&mut self, id: u32) {
        let index = id as usize;
        if index >= self.counts.len() {
            self.grow(index);
        }
        // SAFETY: `grow` made `index` in bounds.
        let count = unsafe { self.counts.get_unchecked_mut(index) };
        if *count == 0 {
            self.touched.push(id);
        }
        // AFL NeverZero: a counter that wraps stays non-zero.
        *count = count.wrapping_add(1).max(1);
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self, index: usize) {
        self.counts.resize((index + 1).next_power_of_two(), 0);
    }

    fn reset(&mut self) {
        for id in self.touched.drain(..) {
            self.counts[id as usize] = 0;
        }
    }

    fn take(&mut self) -> Vec<InstrumentedEdgeHit> {
        let Self { counts, touched } = self;
        touched
            .drain(..)
            .map(|id| {
                let count = std::mem::take(&mut counts[id as usize]);
                InstrumentedEdgeHit { id, count }
            })
            .collect()
    }
}

/// Comparison operand log of one call for compiler-instrumented sites, capped as the cmp-log
/// inspector caps it, but counted in a table indexed by the dense site ID.
///
/// Sites are those of the bytecode, so copies of one contract at several addresses share their
/// caps; each observation still records the address it ran at.
#[derive(Debug, Default)]
struct SiteCmpLog {
    log: Vec<CmpOperands>,
    /// Observations per site, indexed by site ID, grown on demand.
    counts: Vec<u8>,
    /// Sites observed since the last reset.
    touched: Vec<u32>,
}

impl SiteCmpLog {
    const fn new() -> Self {
        Self { log: Vec::new(), counts: Vec::new(), touched: Vec::new() }
    }

    #[inline(always)]
    fn record(&mut self, site: u32, cmp: impl FnOnce() -> CmpOperands) {
        let index = site as usize;
        if index >= self.counts.len() {
            self.grow(index);
        }
        // SAFETY: `grow` made `index` in bounds.
        let count = unsafe { self.counts.get_unchecked_mut(index) };
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

    #[cold]
    #[inline(never)]
    fn grow(&mut self, index: usize) {
        self.counts.resize((index + 1).next_power_of_two(), 0);
    }

    fn reset_counts(&mut self) {
        for site in self.touched.drain(..) {
            self.counts[site as usize] = 0;
        }
    }

    fn clear(&mut self) {
        self.reset_counts();
        self.log.clear();
    }

    fn take(&mut self) -> Vec<CmpOperands> {
        self.reset_counts();
        std::mem::take(&mut self.log)
    }
}

/// A mapping slot observation, in execution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingEvent {
    /// A 64-byte `KECCAK256` in the storage of `address`: `result = keccak256(key ++ parent)`.
    Hash { address: Address, result: B256, key: B256, parent: B256 },
    /// An `SSTORE` to `slot` in the storage of `address`.
    Store { address: Address, slot: B256 },
}

thread_local! {
    // `UnsafeCell`s rather than `RefCell`s keep the `EDGECOV` and `CMPLOG` hot paths free of
    // borrow-flag updates. Every access goes through `with_edge_hits` or `with_cmp_log`, whose
    // closures never re-enter them.
    static EDGE_HITS: UnsafeCell<EdgeHits> = const { UnsafeCell::new(EdgeHits::new()) };
    static CMP_LOG: UnsafeCell<SiteCmpLog> = const { UnsafeCell::new(SiteCmpLog::new()) };
    static FRAME_STACK: RefCell<Option<Vec<U256>>> = const { RefCell::new(None) };
    static MAPPING_EVENTS: RefCell<Vec<MappingEvent>> = const { RefCell::new(Vec::new()) };
}

#[inline(always)]
fn with_edge_hits<R>(f: impl FnOnce(&mut EdgeHits) -> R) -> R {
    // SAFETY: the reference does not escape `f`, and no caller re-enters this function from
    // inside `f`, so it is the only live reference to this thread's map.
    EDGE_HITS.with(|hits| f(unsafe { &mut *hits.get() }))
}

#[inline(always)]
fn with_cmp_log<R>(f: impl FnOnce(&mut SiteCmpLog) -> R) -> R {
    // SAFETY: as in `with_edge_hits`.
    CMP_LOG.with(|log| f(unsafe { &mut *log.get() }))
}

/// Records one hit of edge `id` on this thread. IDs 0 and above [`MAX_EDGE_ID`] are ignored.
#[inline(always)]
pub fn record_edge(id: U256) {
    let id = match u32::try_from(id) {
        Ok(id @ 1..=MAX_EDGE_ID) => id,
        _ => return,
    };
    with_edge_hits(|hits| hits.record(id));
}

/// Clears this thread's hits, cmp-log, pending frame stack and mapping events before a call, in
/// time proportional to what the previous call recorded.
pub fn reset_edges() {
    with_edge_hits(EdgeHits::reset);
    with_cmp_log(SiteCmpLog::clear);
    FRAME_STACK.with_borrow_mut(Option::take);
    MAPPING_EVENTS.with_borrow_mut(Vec::clear);
}

/// Takes this thread's hits in first-hit order and clears them.
pub fn take_edges() -> Vec<InstrumentedEdgeHit> {
    with_edge_hits(EdgeHits::take)
}

/// Takes this thread's comparison operands recorded since the last reset.
pub fn take_cmp_log() -> Vec<CmpOperands> {
    with_cmp_log(SiteCmpLog::take)
}

/// Passes this thread's mapping events recorded since they were last taken to `f`, in execution
/// order, and clears them.
pub fn take_mapping_events(mut f: impl FnMut(MappingEvent)) {
    let events = MAPPING_EVENTS.with_borrow_mut(std::mem::take);
    if !events.is_empty() {
        events.into_iter().for_each(&mut f);
    }
}

/// Takes the caller's stack recorded by the `CALL`/`CREATE`-family instruction that opened the
/// frame now starting, if one did (a transaction's root frame has none).
///
/// The stack is as the instruction left it, with its operands popped: once the frame returns,
/// the caller resumes with this stack plus the frame's result word.
pub fn take_pending_frame_stack() -> Option<Vec<U256>> {
    FRAME_STACK.with_borrow_mut(Option::take)
}

/// The `EDGECOV` instruction: pops the edge ID, records the hit and refunds the `PUSH` gas.
#[inline]
pub fn edgecov<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let Some(id) = context.interpreter.stack.pop() else {
        return Err(InstructionResult::StackUnderflow);
    };
    context.interpreter.gas.erase_cost(PUSH_GAS);
    record_edge(id);
    Ok(())
}

/// Runs revm's default instruction for `OP`.
#[inline(always)]
fn run_default<const OP: u8, H: Host>(
    context: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    instruction_table::<EthInterpreter, H>()[OP as usize].execute(context)
}

/// The `CMPLOG` instruction: pops `site << 8 | cmp opcode`, records the two words below it as
/// the operands of that comparison, as the comparison will read them, and refunds the `PUSH` gas.
///
/// The site ID stands for the comparison's program counter in [`CmpOperands::pc`]: it is unique
/// per comparison of the compilation, and stable across builds compiled the same way.
#[inline]
pub fn cmplog<H: ?Sized>(
    context: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    let interpreter = context.interpreter;
    let tag = interpreter.stack.pop()?;
    let (Ok(op1), Ok(op2)) = (interpreter.stack.peek(0), interpreter.stack.peek(1)) else {
        return Err(InstructionResult::StackUnderflow);
    };
    interpreter.gas.erase_cost(PUSH_GAS);
    let Ok(tag) = u64::try_from(tag) else { return Ok(()) };
    let kind = tag as u8;
    let site = match u32::try_from(tag >> 8) {
        Ok(site @ 1..=MAX_EDGE_ID) => site,
        _ => return Ok(()),
    };
    if !matches!(kind, opcode::EQ | opcode::LT | opcode::GT | opcode::SLT | opcode::SGT) {
        return Ok(());
    }
    let address = interpreter.input.target_address();
    with_cmp_log(|log| {
        log.record(site, || CmpOperands { op1, op2, pc: site as usize, address, opcode: kind })
    });
    Ok(())
}

/// The `CMPLOG` instruction of an EVM that records no comparisons: pops the site and refunds the
/// `PUSH` gas.
fn skip_cmplog<H: ?Sized>(
    context: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    if context.interpreter.stack.pop().is_err() || context.interpreter.stack.len() < 2 {
        return Err(InstructionResult::StackUnderflow);
    }
    context.interpreter.gas.erase_cost(PUSH_GAS);
    Ok(())
}

/// A `KECCAK256` instruction that records a 64-byte preimage as a [`MappingEvent::Hash`] once it
/// succeeds, as the fuzzer's mapping slot tracking reads it around the instruction.
fn keccak_hook<H: Host>(
    context: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    let InstructionContext { interpreter, host } = context;
    let offset = (interpreter.stack.peek(1).ok() == Some(U256::from(0x40)))
        .then(|| interpreter.stack.peek(0).ok().and_then(|offset| usize::try_from(offset).ok()))
        .flatten();
    let result = run_default::<{ opcode::KECCAK256 }, H>(InstructionContext {
        interpreter: &mut *interpreter,
        host,
    });
    if let Some(offset) = offset
        && result.is_ok()
        && interpreter.bytecode.action.is_none()
        && let Ok(hash) = interpreter.stack.peek(0)
    {
        let data = interpreter.memory.slice_len(offset, 0x40);
        let event = MappingEvent::Hash {
            address: interpreter.input.target_address(),
            result: hash.into(),
            key: B256::from_slice(&data[..0x20]),
            parent: B256::from_slice(&data[0x20..]),
        };
        drop(data);
        MAPPING_EVENTS.with_borrow_mut(|events| events.push(event));
    }
    result
}

/// An `SSTORE` instruction that records its slot as a [`MappingEvent::Store`], then executes it.
fn sstore_hook<H: Host>(
    context: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    if let Ok(slot) = context.interpreter.stack.peek(0) {
        let event = MappingEvent::Store {
            address: context.interpreter.input.target_address(),
            slot: slot.into(),
        };
        MAPPING_EVENTS.with_borrow_mut(|events| events.push(event));
    }
    run_default::<{ opcode::SSTORE }, H>(context)
}

/// A `CALL`/`CREATE`-family instruction that, when it opens a frame, records the caller's
/// stack for [`take_pending_frame_stack`].
fn frame_hook<const OP: u8, H: Host>(
    context: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    let InstructionContext { interpreter, host } = context;
    let result = run_default::<OP, H>(InstructionContext { interpreter: &mut *interpreter, host });
    // An instruction that opens a frame suspends the interpreter with the frame as its action.
    if matches!(interpreter.bytecode.action, Some(InterpreterAction::NewFrame(_))) {
        let stack = interpreter.stack.data().clone();
        FRAME_STACK.with_borrow_mut(|pending| *pending = Some(stack));
    }
    result
}

/// Installs the hooks `hooks` asks for into an instruction table, keeping each wrapped opcode's
/// static gas.
pub fn install_hooks<CTX: Host>(
    instructions: &mut EthInstructions<EthInterpreter, CTX>,
    hooks: EvmHooks,
) {
    if hooks.edgecov {
        instructions.insert_instruction(EDGECOV, Instruction::new(edgecov), 0);
    }
    let mut wrap = |op: u8, instruction: Instruction<EthInterpreter, CTX>| {
        let gas = instructions.gas_table()[op as usize];
        instructions.insert_instruction(op, instruction, gas);
    };
    // Instrumented bytecode carries both probes, so an EVM that runs it executes `CMPLOG` even
    // when it records no comparisons.
    if hooks.cmp_log {
        wrap(CMPLOG, Instruction::new(cmplog));
    } else if hooks.edgecov {
        wrap(CMPLOG, Instruction::new(skip_cmplog));
    }
    if hooks.mapping_slots {
        wrap(opcode::KECCAK256, Instruction::new(keccak_hook::<CTX>));
        wrap(opcode::SSTORE, Instruction::new(sstore_hook::<CTX>));
    }
    if hooks.frame_stacks {
        wrap(opcode::CALL, Instruction::new(frame_hook::<{ opcode::CALL }, CTX>));
        wrap(opcode::CALLCODE, Instruction::new(frame_hook::<{ opcode::CALLCODE }, CTX>));
        wrap(opcode::DELEGATECALL, Instruction::new(frame_hook::<{ opcode::DELEGATECALL }, CTX>));
        wrap(opcode::STATICCALL, Instruction::new(frame_hook::<{ opcode::STATICCALL }, CTX>));
        wrap(opcode::CREATE, Instruction::new(frame_hook::<{ opcode::CREATE }, CTX>));
        wrap(opcode::CREATE2, Instruction::new(frame_hook::<{ opcode::CREATE2 }, CTX>));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(ids: &[u32]) -> Vec<InstrumentedEdgeHit> {
        reset_edges();
        for &id in ids {
            record_edge(U256::from(id));
        }
        take_edges()
    }

    #[test]
    fn counts_hits_in_first_hit_order() {
        assert_eq!(
            hits(&[7, 2, 7, 7]),
            [InstrumentedEdgeHit { id: 7, count: 3 }, InstrumentedEdgeHit { id: 2, count: 1 }]
        );
        // Taking clears the map.
        assert!(take_edges().is_empty());
    }

    #[test]
    fn counter_never_wraps_to_zero() {
        let ids = vec![5; 256];
        assert_eq!(hits(&ids), [InstrumentedEdgeHit { id: 5, count: 1 }]);
        let ids = vec![5; 255];
        assert_eq!(hits(&ids), [InstrumentedEdgeHit { id: 5, count: 255 }]);
    }

    #[test]
    fn reset_clears_previous_hits() {
        record_edge(U256::from(3));
        record_edge(U256::from(1000));
        reset_edges();
        assert!(take_edges().is_empty());
        assert_eq!(hits(&[3]), [InstrumentedEdgeHit { id: 3, count: 1 }]);
    }

    #[test]
    fn ignores_reserved_and_out_of_range_ids() {
        assert!(hits(&[0, MAX_EDGE_ID + 1]).is_empty());
        reset_edges();
        record_edge(U256::MAX);
        assert!(take_edges().is_empty());
    }
}
