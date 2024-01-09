use revm::{
    interpreter::{opcode, opcode::spec_opcode_gas},
    primitives::{HashMap, SpecId},
};

/// Maps from program counter to instruction counter.
///
/// Inverse of [`IcPcMap`].
pub struct PcIcMap {
    pub inner: HashMap<usize, usize>,
}

impl PcIcMap {
    /// Creates a new `PcIcMap` for the given code.
    pub fn new(spec: SpecId, code: &[u8]) -> Self {
        let opcode_infos = spec_opcode_gas(spec);
        let mut map = HashMap::new();

        let mut i = 0;
        let mut cumulative_push_size = 0;
        while i < code.len() {
            let op = code[i];
            map.insert(i, i - cumulative_push_size);
            if opcode_infos[op as usize].is_push() {
                // Skip the push bytes.
                //
                // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
                i += (op - opcode::PUSH1 + 1) as usize;
                cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
            }
            i += 1;
        }

        Self { inner: map }
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
    pub inner: HashMap<usize, usize>,
}

impl IcPcMap {
    /// Creates a new `IcPcMap` for the given code.
    pub fn new(spec: SpecId, code: &[u8]) -> Self {
        let opcode_infos = spec_opcode_gas(spec);
        let mut map = HashMap::new();

        let mut i = 0;
        let mut cumulative_push_size = 0;
        while i < code.len() {
            let op = code[i];
            map.insert(i - cumulative_push_size, i);
            if opcode_infos[op as usize].is_push() {
                // Skip the push bytes.
                //
                // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
                i += (op - opcode::PUSH1 + 1) as usize;
                cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
            }
            i += 1;
        }

        Self { inner: map }
    }

    /// Returns the program counter for the given instruction counter.
    pub fn get(&self, ic: usize) -> Option<usize> {
        self.inner.get(&ic).copied()
    }
}
