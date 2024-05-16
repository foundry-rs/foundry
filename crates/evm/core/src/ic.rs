use revm::{
    interpreter::{opcode, OpCode, OPCODE_INFO_JUMPTABLE},
    primitives::SpecId,
};
use rustc_hash::FxHashMap;

/// Maps from program counter to instruction counter.
///
/// Inverse of [`IcPcMap`].
pub struct PcIcMap {
    pub inner: FxHashMap<usize, usize>,
}

impl PcIcMap {
    /// Creates a new `PcIcMap` for the given code.
    pub fn new(spec: SpecId, code: &[u8]) -> Self {
        Self { inner: make_map::<true>(spec, code) }
    }

    /// Returns the instruction counter for the given program counter.
    pub fn get(&self, pc: usize) -> Option<usize> {
        self.inner.get(&pc).copied()
    }
}

/// Map from instruction counter to program counter.
///
/// Inverse of [`PcIcMap`].
pub struct IcPcMap {
    pub inner: FxHashMap<usize, usize>,
}

impl IcPcMap {
    /// Creates a new `IcPcMap` for the given code.
    pub fn new(spec: SpecId, code: &[u8]) -> Self {
        Self { inner: make_map::<false>(spec, code) }
    }

    /// Returns the program counter for the given instruction counter.
    pub fn get(&self, ic: usize) -> Option<usize> {
        self.inner.get(&ic).copied()
    }
}

fn make_map<const PC_FIRST: bool>(_spec: SpecId, code: &[u8]) -> FxHashMap<usize, usize> {
    let mut map = FxHashMap::default();

    let mut pc = 0;
    let mut cumulative_push_size = 0;
    while pc < code.len() {
        let ic = pc - cumulative_push_size;
        if PC_FIRST {
            map.insert(pc, ic);
        } else {
            map.insert(ic, pc);
        }

        let op = unsafe { OpCode::new_unchecked(code[pc]) };
        if op.is_push() {
            // Skip the push bytes.
            let push_size = (op.get() - opcode::PUSH0) as usize;
            pc += push_size;
            cumulative_push_size += push_size;
        }

        pc += 1;
    }
    map
}
