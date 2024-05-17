use revm::interpreter::{
    opcode::{PUSH0, PUSH1, PUSH32},
    OPCODE_INFO_JUMPTABLE,
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
    pub fn new(code: &[u8]) -> Self {
        Self { inner: make_map::<true>(code) }
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
    pub fn new(code: &[u8]) -> Self {
        Self { inner: make_map::<false>(code) }
    }

    /// Returns the program counter for the given instruction counter.
    pub fn get(&self, ic: usize) -> Option<usize> {
        self.inner.get(&ic).copied()
    }
}

fn make_map<const PC_FIRST: bool>(code: &[u8]) -> FxHashMap<usize, usize> {
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

        let _op_info = OPCODE_INFO_JUMPTABLE[code[pc] as usize]
            .ok_or_else(|| eyre::eyre!("Invalid opcode: {}, Not found in jump table", code[pc]));
        if code[pc] >= PUSH1 && code[pc] <= PUSH32 {
            // Skip the push bytes.
            let push_size = (code[pc] - PUSH0) as usize;
            pc += push_size;
            cumulative_push_size += push_size;
        }

        pc += 1;
    }
    map
}
