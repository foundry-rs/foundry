//! Branch-trace inspector for the symbolic-assist worker.
//!
//! Records every conditional jump (`JUMPI`) the EVM executes, together with the
//! comparator operands of the immediately preceding compare opcode
//! (`EQ` / `LT` / `GT` / `SLT` / `SGT` / `ISZERO`) when present. The resulting
//! [`BranchTrace`] is consumed by the symbolic-assist worker
//! (`crate::executors::symexec`) to:
//!
//! 1. find "frontier" branches whose opposite edge has never been covered, and
//! 2. propose ABI-aware mutations of the calldata that would flip the branch.
//!
//! This is the first piece of the concolic-lite engine described in the
//! architectural plan: it captures the same information AFL/Redqueen and
//! libFuzzer's `trace_cmp` use, but at the EVM level.
//!
//! NOTE: this inspector is intentionally *observational* only. It must not
//! mutate any EVM state and must be safe to run alongside the normal
//! `EdgeCovInspector`.

use alloy_primitives::{Address, U256, map::DefaultHashBuilder};
use core::hash::{BuildHasher, Hash, Hasher};
use revm::{
    Inspector,
    bytecode::opcode,
    interpreter::{
        Interpreter,
        interpreter_types::{InputsTr, Jumps},
    },
};

/// Must match `MAX_EDGE_COUNT` in `crate::inspectors::edge_cov`.
/// Kept here as a private constant so the symbolic worker can compare its
/// frontier ids against the corpus `history_map` without depending on
/// `EdgeCovInspector` directly. (Refactor target: extract a shared
/// `edge_id(address, pc, jump_dest)` helper.)
const MAX_EDGE_COUNT: usize = 65536;

/// A compare opcode observed immediately before a `JUMPI`.
///
/// We capture the concrete operands so the symbolic worker can derive a
/// targeted mutation (e.g. "set this calldata word to `rhs`") without invoking
/// an SMT solver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpKind {
    Eq,
    Lt,
    Gt,
    Slt,
    Sgt,
    /// `ISZERO` operates on a single operand; `rhs` is unused.
    IsZero,
}

/// A single compare observation.
#[derive(Clone, Copy, Debug)]
pub struct CmpObservation {
    pub kind: CmpKind,
    pub lhs: U256,
    /// `U256::ZERO` for `ISZERO`.
    pub rhs: U256,
}

/// One observed `JUMPI` along the executed path.
#[derive(Clone, Debug)]
pub struct BranchObservation {
    /// Contract whose bytecode contains the `JUMPI`.
    pub address: Address,
    /// Program counter of the `JUMPI` instruction.
    pub pc: usize,
    /// Destination if the branch was taken.
    pub taken_dest: U256,
    /// Destination if the branch was *not* taken (always `pc + 1`).
    pub other_dest: U256,
    /// Whether the branch was taken on this trace.
    pub took_branch: bool,
    /// Concrete compare that produced the branch condition, when the
    /// instruction immediately before the `JUMPI` was a recognised compare.
    pub cmp: Option<CmpObservation>,
}

impl BranchObservation {
    /// Hashed edge id of the destination *not* taken on this trace, computed
    /// the same way as `EdgeCovInspector::store_hit`. The symbolic worker
    /// uses this to look up whether the opposite side is unseen in the
    /// corpus' `history_map`.
    pub fn frontier_edge_id(&self, hash_builder: &DefaultHashBuilder) -> usize {
        let dest = if self.took_branch { self.other_dest } else { self.taken_dest };
        edge_id(hash_builder, self.address, self.pc, dest)
    }
}

/// Helper mirroring the hashing in `EdgeCovInspector::store_hit`.
///
/// Kept private here for now; should be hoisted to a shared module once we
/// decide on the canonical location.
fn edge_id(
    hash_builder: &DefaultHashBuilder,
    address: Address,
    pc: usize,
    jump_dest: U256,
) -> usize {
    let mut hasher = hash_builder.build_hasher();
    address.hash(&mut hasher);
    pc.hash(&mut hasher);
    jump_dest.hash(&mut hasher);
    (hasher.finish() % MAX_EDGE_COUNT as u64) as usize
}

/// All branch observations recorded for a single execution.
#[derive(Clone, Debug, Default)]
pub struct BranchTrace {
    pub branches: Vec<BranchObservation>,
}

impl BranchTrace {
    pub const fn new() -> Self {
        Self { branches: Vec::new() }
    }

    pub fn clear(&mut self) {
        self.branches.clear();
    }

    pub fn len(&self) -> usize {
        self.branches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }
}

/// `Inspector` that records [`BranchObservation`]s for the symbolic worker.
#[derive(Clone, Debug, Default)]
pub struct BranchTraceInspector {
    trace: BranchTrace,
    /// Last compare opcode observed. Cleared after the next instruction.
    /// Tracking only the immediately-preceding compare keeps this cheap and
    /// matches Solidity's typical `cmp; iszero?; PUSH dest; JUMPI` pattern.
    pending_cmp: Option<CmpObservation>,
    /// If the instruction before `JUMPI` is `ISZERO`, the underlying compare
    /// (if any) was two opcodes back — we keep both.
    prev_pending_cmp: Option<CmpObservation>,
}

impl BranchTraceInspector {
    pub const fn new() -> Self {
        Self { trace: BranchTrace::new(), pending_cmp: None, prev_pending_cmp: None }
    }

    pub const fn trace(&self) -> &BranchTrace {
        &self.trace
    }

    pub fn take_trace(&mut self) -> BranchTrace {
        core::mem::take(&mut self.trace)
    }

    pub fn reset(&mut self) {
        self.trace.clear();
        self.pending_cmp = None;
        self.prev_pending_cmp = None;
    }

    fn record_cmp(&mut self, kind: CmpKind, interp: &Interpreter) {
        // Operand order on the EVM stack: top = a, next = b.
        // Comparisons compute `a OP b`; `ISZERO` just consumes `a`.
        let lhs = interp.stack.peek(0).unwrap_or(U256::ZERO);
        let rhs = if matches!(kind, CmpKind::IsZero) {
            U256::ZERO
        } else {
            interp.stack.peek(1).unwrap_or(U256::ZERO)
        };
        // Shift the previous pending compare down so we can recover it past an
        // intervening `ISZERO`.
        self.prev_pending_cmp = self.pending_cmp.take();
        self.pending_cmp = Some(CmpObservation { kind, lhs, rhs });
    }

    fn record_jumpi(&mut self, interp: &Interpreter) {
        let address = interp.input.target_address();
        let pc = interp.bytecode.pc();

        // Stack layout for JUMPI: [0]=dest, [1]=condition.
        let Ok(dest) = interp.stack.peek(0) else { return };
        let Ok(cond) = interp.stack.peek(1) else { return };

        let took_branch = !cond.is_zero();
        let taken_dest = if took_branch { dest } else { U256::from(pc + 1) };
        let other_dest = if took_branch { U256::from(pc + 1) } else { dest };

        // Prefer the underlying compare if the immediate predecessor was
        // `ISZERO` (Solidity's `if (x == y)` becomes `EQ; ISZERO; PUSH; JUMPI`
        // and its negation `EQ; PUSH; JUMPI`).
        let cmp = match self.pending_cmp {
            Some(c) if c.kind == CmpKind::IsZero => self.prev_pending_cmp,
            other => other,
        };

        self.trace.branches.push(BranchObservation {
            address,
            pc,
            taken_dest,
            other_dest,
            took_branch,
            cmp,
        });
    }
}

impl<CTX> Inspector<CTX> for BranchTraceInspector {
    #[inline]
    fn step(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        let op = interp.bytecode.opcode();
        match op {
            opcode::EQ => self.record_cmp(CmpKind::Eq, interp),
            opcode::LT => self.record_cmp(CmpKind::Lt, interp),
            opcode::GT => self.record_cmp(CmpKind::Gt, interp),
            opcode::SLT => self.record_cmp(CmpKind::Slt, interp),
            opcode::SGT => self.record_cmp(CmpKind::Sgt, interp),
            opcode::ISZERO => self.record_cmp(CmpKind::IsZero, interp),
            opcode::JUMPI => {
                self.record_jumpi(interp);
                // Reset pending compares — we only care about the cmp that
                // produced *this* branch's condition.
                self.pending_cmp = None;
                self.prev_pending_cmp = None;
            }
            _ => {
                // Any other opcode invalidates the pending-compare window.
                if self.pending_cmp.is_some() {
                    self.prev_pending_cmp = None;
                    self.pending_cmp = None;
                }
            }
        }
    }
}
