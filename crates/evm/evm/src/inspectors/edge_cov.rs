use alloy_primitives::{Address, U256, map::DefaultHashBuilder};
use core::{
    fmt,
    hash::{BuildHasher, Hash, Hasher},
};
use revm::{
    Inspector,
    bytecode::opcode,
    interpreter::{
        Interpreter,
        interpreter_types::{InputsTr, Jumps},
    },
};

// This is the maximum number of edges that can be tracked. There is a tradeoff between performance
// and precision (less collisions).
const MAX_EDGE_COUNT: usize = 65536;

// Maximum number of comparison operand pairs to track for CmpLog-style feedback.
const MAX_CMP_LOG_ENTRIES: usize = 1024;

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
    /// Comparison kind.
    pub kind: CmpKind,
    /// Whether the comparison interprets operands as signed integers.
    pub signed: bool,
    /// Inferred operand width in bits, or zero when unknown.
    pub width: u16,
}

/// EVM comparison kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CmpKind {
    /// Equality comparison.
    #[default]
    Eq,
    /// Less-than comparison.
    Lt,
    /// Greater-than comparison.
    Gt,
    /// Zero comparison.
    IsZero,
}

/// An `Inspector` that tracks [edge coverage](https://clang.llvm.org/docs/SanitizerCoverage.html#edge-coverage).
/// Covered edges will not wrap to zero e.g. a loop edge hit more than 255 will still be retained.
///
/// Also tracks comparison operands for CmpLog-style guided fuzzing.
// see https://github.com/AFLplusplus/AFLplusplus/blob/5777ceaf23f48ae4ceae60e4f3a79263802633c6/instrumentation/afl-llvm-pass.so.cc#L810-L829
#[derive(Clone)]
pub struct EdgeCovInspector {
    /// Map of hitcounts that can be diffed against to determine if new coverage was reached.
    hitcount: Vec<u8>,
    hash_builder: DefaultHashBuilder,
    /// Comparison operand log for CmpLog-style guided fuzzing.
    cmp_log: Option<Vec<CmpOperands>>,
}

impl fmt::Debug for EdgeCovInspector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EdgeCovInspector").finish_non_exhaustive()
    }
}

impl EdgeCovInspector {
    /// Create a new `EdgeCovInspector` with `MAX_EDGE_COUNT` size.
    pub fn new() -> Self {
        Self {
            hitcount: vec![0; MAX_EDGE_COUNT],
            hash_builder: DefaultHashBuilder::default(),
            cmp_log: None,
        }
    }

    /// Create a new `EdgeCovInspector` with comparison operand logging enabled.
    pub fn with_cmp_log() -> Self {
        let mut inspector = Self::new();
        inspector.enable_cmp_log(true);
        inspector
    }

    /// Set whether to collect comparison operand logs.
    pub fn enable_cmp_log(&mut self, yes: bool) {
        if yes {
            self.cmp_log.get_or_insert_with(|| Vec::with_capacity(MAX_CMP_LOG_ENTRIES));
        } else {
            self.cmp_log = None;
        }
    }

    /// Reset the hitcount to zero and clear the comparison log.
    pub fn reset(&mut self) {
        self.hitcount.fill(0);
        if let Some(cmp_log) = &mut self.cmp_log {
            cmp_log.clear();
        }
    }

    /// Get an immutable reference to the hitcount.
    pub const fn get_hitcount(&self) -> &[u8] {
        self.hitcount.as_slice()
    }

    /// Get an immutable reference to the comparison operand log.
    pub const fn get_cmp_log(&self) -> &[CmpOperands] {
        match &self.cmp_log {
            Some(cmp_log) => cmp_log.as_slice(),
            None => &[],
        }
    }

    /// Consume the inspector and take ownership of the hitcount.
    pub fn into_hitcount(self) -> Vec<u8> {
        self.hitcount
    }

    /// Consume the inspector and take ownership of both the hitcount and comparison log.
    pub fn into_parts(self) -> (Vec<u8>, Vec<CmpOperands>) {
        (self.hitcount, self.cmp_log.unwrap_or_default())
    }

    /// Mark the edge, H(address, pc, jump_dest), as hit.
    fn store_hit(&mut self, address: Address, pc: usize, jump_dest: U256) {
        let mut hasher = self.hash_builder.build_hasher();
        address.hash(&mut hasher);
        pc.hash(&mut hasher);
        jump_dest.hash(&mut hasher);
        // The hash is used to index into the hitcount array,
        // so it must be modulo the maximum edge count.
        let edge_id = (hasher.finish() % MAX_EDGE_COUNT as u64) as usize;
        self.hitcount[edge_id] = self.hitcount[edge_id].checked_add(1).unwrap_or(1);
    }

    /// Store comparison operands for CmpLog-style guided fuzzing.
    fn store_cmp(
        &mut self,
        address: Address,
        pc: usize,
        opcode: u8,
        kind: CmpKind,
        signed: bool,
        op1: U256,
        op2: U256,
    ) {
        if let Some(cmp_log) = &mut self.cmp_log
            && cmp_log.len() < MAX_CMP_LOG_ENTRIES
        {
            cmp_log.push(CmpOperands { op1, op2, pc, address, opcode, kind, signed, width: 0 });
        }
    }

    #[cold]
    fn do_step(&mut self, interp: &mut Interpreter) {
        let address = interp.input.target_address();
        let current_pc = interp.bytecode.pc();

        match interp.bytecode.opcode() {
            opcode::JUMP => {
                // unconditional jump
                if let Ok(jump_dest) = interp.stack.peek(0) {
                    self.store_hit(address, current_pc, jump_dest);
                }
            }
            opcode::JUMPI => {
                if let Ok(stack_value) = interp.stack.peek(1) {
                    let jump_dest = if stack_value.is_zero() {
                        // fall through
                        Ok(U256::from(current_pc + 1))
                    } else {
                        // branch taken
                        interp.stack.peek(0)
                    };

                    if let Ok(jump_dest) = jump_dest {
                        self.store_hit(address, current_pc, jump_dest);
                    }
                }
            }
            _ => {
                // no-op
            }
        }
    }

    #[cold]
    fn do_cmp_step(&mut self, interp: &mut Interpreter) {
        if self.cmp_log.is_none() {
            return;
        }

        let address = interp.input.target_address();
        let current_pc = interp.bytecode.pc();

        match interp.bytecode.opcode() {
            op @ opcode::EQ => {
                if let (Ok(op1), Ok(op2)) = (interp.stack.peek(0), interp.stack.peek(1)) {
                    self.store_cmp(address, current_pc, op, CmpKind::Eq, false, op1, op2);
                }
            }
            op @ (opcode::LT | opcode::SLT) => {
                if let (Ok(op1), Ok(op2)) = (interp.stack.peek(0), interp.stack.peek(1)) {
                    self.store_cmp(
                        address,
                        current_pc,
                        op,
                        CmpKind::Lt,
                        op == opcode::SLT,
                        op1,
                        op2,
                    );
                }
            }
            op @ (opcode::GT | opcode::SGT) => {
                if let (Ok(op1), Ok(op2)) = (interp.stack.peek(0), interp.stack.peek(1)) {
                    self.store_cmp(
                        address,
                        current_pc,
                        op,
                        CmpKind::Gt,
                        op == opcode::SGT,
                        op1,
                        op2,
                    );
                }
            }
            op @ opcode::ISZERO => {
                if let Ok(op1) = interp.stack.peek(0) {
                    self.store_cmp(
                        address,
                        current_pc,
                        op,
                        CmpKind::IsZero,
                        false,
                        op1,
                        U256::ZERO,
                    );
                }
            }
            _ => {}
        }
    }
}

impl Default for EdgeCovInspector {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX> Inspector<CTX> for EdgeCovInspector {
    #[inline]
    fn step(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        let op = interp.bytecode.opcode();
        if matches!(op, opcode::JUMP | opcode::JUMPI) {
            self.do_step(interp);
        }
        if matches!(
            op,
            opcode::EQ | opcode::LT | opcode::GT | opcode::SLT | opcode::SGT | opcode::ISZERO
        ) {
            self.do_cmp_step(interp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmp_operands_defaults_and_clones() {
        let cmp = CmpOperands {
            op1: U256::from(123),
            op2: U256::from(456),
            pc: 42,
            address: Address::repeat_byte(0xaa),
            opcode: opcode::EQ,
            kind: CmpKind::Eq,
            signed: false,
            width: 0,
        };

        assert_eq!(cmp.op1, U256::from(123));
        assert_eq!(cmp.op2, U256::from(456));
        assert_eq!(cmp.pc, 42);

        assert_eq!(CmpOperands::default().kind, CmpKind::Eq);
        let cloned = cmp;
        assert_eq!(cloned, cmp);
    }

    #[test]
    fn cmp_log_starts_empty_and_is_returned_by_into_parts() {
        let inspector = EdgeCovInspector::new();

        assert!(inspector.get_cmp_log().is_empty());
        assert!(inspector.get_hitcount().iter().all(|&x| x == 0));

        let (hitcount, cmp_log) = inspector.into_parts();
        assert_eq!(hitcount.len(), MAX_EDGE_COUNT);
        assert!(cmp_log.is_empty());
    }

    #[test]
    fn reset_clears_hitcount_and_cmp_log() {
        let mut inspector = EdgeCovInspector::with_cmp_log();

        inspector.hitcount[0] = 1;
        inspector.store_cmp(
            Address::ZERO,
            42,
            opcode::EQ,
            CmpKind::Eq,
            false,
            U256::from(123),
            U256::from(456),
        );

        inspector.reset();

        assert!(inspector.get_hitcount().iter().all(|&x| x == 0));
        assert!(inspector.get_cmp_log().is_empty());
    }

    #[test]
    fn cmp_log_is_capped() {
        let mut inspector = EdgeCovInspector::with_cmp_log();

        for i in 0..MAX_CMP_LOG_ENTRIES + 1 {
            inspector.store_cmp(
                Address::ZERO,
                i,
                opcode::EQ,
                CmpKind::Eq,
                false,
                U256::from(i),
                U256::from(i + 1),
            );
        }

        assert_eq!(inspector.get_cmp_log().len(), MAX_CMP_LOG_ENTRIES);
        assert_eq!(inspector.get_cmp_log()[MAX_CMP_LOG_ENTRIES - 1].pc, MAX_CMP_LOG_ENTRIES - 1);
    }
}
