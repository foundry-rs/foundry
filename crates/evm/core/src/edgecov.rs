//! Compiler-instrumented edge coverage through the `EDGECOV` opcode.
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

use alloy_primitives::U256;
use revm::{
    handler::instructions::EthInstructions,
    interpreter::{
        Host, Instruction, InstructionContext, InstructionExecResult, InstructionResult,
        interpreter::EthInterpreter,
        interpreter_types::{InterpreterTypes, StackTr},
    },
};
use std::cell::RefCell;

/// The `EDGECOV` opcode byte.
pub const EDGECOV: u8 = 0xC0;

/// Gas of the `PUSHn` that precedes every `EDGECOV`, refunded by the instruction.
const PUSH_GAS: u64 = 3;

/// Largest edge ID that is recorded. IDs are dense from 1, so anything above this is malformed
/// instrumentation and is ignored instead of growing the map without bound.
pub const MAX_EDGE_ID: u32 = 1 << 24;

/// One instrumented edge hit during a call: its ID and AFL-style hit counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentedEdgeHit {
    /// Compiler-assigned edge ID.
    pub id: u32,
    /// Hit counter; never zero once the edge was hit (it saturates back to 1 on overflow).
    pub count: u8,
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

    #[inline]
    fn record(&mut self, id: u32) {
        let index = id as usize;
        if index >= self.counts.len() {
            self.grow(index);
        }
        let count = &mut self.counts[index];
        if *count == 0 {
            self.touched.push(id);
        }
        // AFL NeverZero: a counter that wraps stays non-zero.
        *count = count.wrapping_add(1).max(1);
    }

    #[cold]
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

thread_local! {
    static EDGE_HITS: RefCell<EdgeHits> = const { RefCell::new(EdgeHits::new()) };
}

/// Records one hit of edge `id` on this thread. IDs 0 and above [`MAX_EDGE_ID`] are ignored.
#[inline]
pub fn record_edge(id: U256) {
    let id = match u32::try_from(id) {
        Ok(id @ 1..=MAX_EDGE_ID) => id,
        _ => return,
    };
    EDGE_HITS.with_borrow_mut(|hits| hits.record(id));
}

/// Clears this thread's hits, in time proportional to the edges hit since the last reset.
pub fn reset_edges() {
    EDGE_HITS.with_borrow_mut(EdgeHits::reset);
}

/// Takes this thread's hits in first-hit order and clears them.
pub fn take_edges() -> Vec<InstrumentedEdgeHit> {
    EDGE_HITS.with_borrow_mut(EdgeHits::take)
}

/// The `EDGECOV` instruction: pops the edge ID, records the hit and refunds the `PUSH` gas.
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

/// Installs the `EDGECOV` instruction, with no static gas, into an instruction table.
pub fn install_edgecov<CTX: Host>(instructions: &mut EthInstructions<EthInterpreter, CTX>) {
    instructions.insert_instruction(EDGECOV, Instruction::new(edgecov), 0);
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
