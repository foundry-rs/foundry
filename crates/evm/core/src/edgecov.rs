//! Compiler-instrumented edge coverage and comparison logging through the `EDGECOV` and `CMPLOG`
//! opcodes.
//!
//! Coverage builds of a compiler emit `PUSHn <id>; EDGECOV` on every control-flow edge and
//! `PUSHn <site << 8 | cmp opcode>; CMPLOG` right before every `EQ`, `LT`, `GT`, `SLT` and `SGT`.
//! Both opcodes are otherwise undefined. Foundry's EVMs carry them in their opcode tables, but
//! they fail as undefined opcodes unless an [`EdgeCovGuard`] is alive on the thread.
//!
//! While it is, `EDGECOV` pops the edge ID and bumps its AFL NeverZero hit counter, and `CMPLOG`
//! pops its tag and records the two words below it, the operands of the comparison that follows.
//! Both refund the 3 gas of the `PUSH` the compiler emits before them, so a probe costs no gas
//! once it ran. Hits and operands are kept per thread in dense tables indexed by ID, and taken
//! once the call ends.

use alloy_primitives::{Address, U256};
use evm2::{
    EvmConfig, EvmTypesHost, ExecutionConfig, OpcodeConfig, SpecId, Version,
    interpreter::{InstrStop, op},
};
use evm2_macros::instruction;
use std::{cell::UnsafeCell, marker::PhantomData};

/// The `EDGECOV` opcode byte.
const EDGECOV: u8 = 0xC0;

/// The `CMPLOG` opcode byte.
const CMPLOG: u8 = 0xC1;

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
    /// Whether the opcodes execute instead of failing as undefined.
    enabled: bool,
    /// Whether `EDGECOV` records hits.
    edges: bool,
    /// Whether `CMPLOG` records operands.
    cmp_log: bool,
    hits: EdgeHits,
    cmps: SiteCmpLog,
}

thread_local! {
    // An `UnsafeCell` rather than a `RefCell` keeps the probes free of borrow-flag updates.
    static STATE: UnsafeCell<State> = const {
        UnsafeCell::new(State {
            enabled: false,
            edges: false,
            cmp_log: false,
            hits: EdgeHits { counts: Vec::new(), touched: Vec::new() },
            cmps: SiteCmpLog { log: Vec::new(), counts: Vec::new(), touched: Vec::new() },
        })
    };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: the reference does not escape `f`, and nothing called from `f` re-enters this
    // function, so it is the only live reference to this thread's state.
    STATE.with(|state| f(unsafe { &mut *state.get() }))
}

/// Executes the instrumentation opcodes on this thread while alive, recording edge hits when
/// `edges` is set and comparison operands when `cmp_log` is set.
#[derive(Debug)]
pub struct EdgeCovGuard(());

impl EdgeCovGuard {
    /// Clears the hits and operands of the previous call and enables the opcodes.
    pub fn new(edges: bool, cmp_log: bool) -> Self {
        with_state(|state| {
            state.enabled = true;
            state.edges = edges;
            state.cmp_log = cmp_log;
            state.hits.take();
            state.cmps.take();
        });
        Self(())
    }
}

impl Drop for EdgeCovGuard {
    fn drop(&mut self) {
        with_state(|state| state.enabled = false);
    }
}

/// Takes this thread's edge hits in first-hit order.
pub fn take_edges() -> Vec<InstrumentedEdgeHit> {
    with_state(|state| state.hits.take())
}

/// Takes this thread's comparison operands.
pub fn take_cmp_log() -> Vec<CmpOperands> {
    with_state(|state| state.cmps.take())
}

/// Converts a probe's ID word, ignoring the reserved ID 0 and IDs above [`MAX_EDGE_ID`].
fn probe_id(word: U256) -> Option<u32> {
    u32::try_from(word).ok().filter(|id| (1..=MAX_EDGE_ID).contains(id))
}

/// The `EDGECOV` instruction: pops the edge ID, records the hit and refunds the `PUSH` gas.
#[instruction(dynamic_gas, no_stack_preamble)]
fn edgecov(cx: _) -> Result {
    with_state(|state| {
        if !state.enabled {
            return Err(InstrStop::OpcodeNotFound);
        }
        let id = stack.pop()?;
        cx.gas.erase_cost(PUSH_GAS);
        if state.edges
            && let Some(id) = probe_id(id)
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
        if !state.enabled {
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

/// Adds the `EDGECOV` and `CMPLOG` instructions to `config`.
pub(crate) const fn with_instrumentation<T: EvmTypesHost>(
    mut config: OpcodeConfig<T>,
) -> OpcodeConfig<T> {
    config.set_instruction::<edgecov<T>>(EDGECOV, 0);
    config.set_instruction::<cmplog<T>>(CMPLOG, 0);
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
        let _guard = EdgeCovGuard::new(true, false);
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
                EDGECOV | CMPLOG => OpcodeChange::Instruction,
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
