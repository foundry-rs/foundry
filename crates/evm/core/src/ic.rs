use alloy_primitives::map::HashMap;
use eyre::Result;
use revm::interpreter::{
    opcode::{PUSH0, PUSH1, PUSH32},
    OpCode,
};
use revm_inspectors::opcode::immediate_size;

/// Maps from program counter to instruction counter.
///
/// Inverse of [`IcPcMap`].
#[derive(Debug, Clone)]
pub struct PcIcMap {
    pub inner: HashMap<usize, usize>,
}

impl PcIcMap {
    /// Creates a new `PcIcMap` for the given code.
    pub fn new(code: &[u8]) -> Self {
        Self { inner: make_map::<true>(code) }
    }

    /// Returns the length of the map.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
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
    pub fn new(code: &[u8]) -> Self {
        Self { inner: make_map::<false>(code) }
    }

    /// Returns the length of the map.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the program counter for the given instruction counter.
    pub fn get(&self, ic: usize) -> Option<usize> {
        self.inner.get(&ic).copied()
    }
}

fn make_map<const PC_FIRST: bool>(code: &[u8]) -> HashMap<usize, usize> {
    let mut map = HashMap::default();

    let mut pc = 0;
    let mut cumulative_push_size = 0;
    while pc < code.len() {
        let ic = pc - cumulative_push_size;
        if PC_FIRST {
            map.insert(pc, ic);
        } else {
            map.insert(ic, pc);
        }

        if (PUSH1..=PUSH32).contains(&code[pc]) {
            // Skip the push bytes.
            let push_size = (code[pc] - PUSH0) as usize;
            pc += push_size;
            cumulative_push_size += push_size;
        }

        pc += 1;
    }
    map
}

/// Represents a single instruction consisting of the opcode and its immediate data.
pub struct Instruction<'a> {
    /// OpCode, if it could be decoded.
    pub op: Option<OpCode>,
    /// Immediate data following the opcode.
    pub immediate: &'a [u8],
    /// Program counter of the opcode.
    pub pc: usize,
}

/// Decodes raw opcode bytes into [`Instruction`]s.
pub fn decode_instructions(code: &[u8]) -> Result<Vec<Instruction<'_>>> {
    let mut pc = 0;
    let mut steps = Vec::new();

    while pc < code.len() {
        let op = OpCode::new(code[pc]);
        let immediate_size = op.map(|op| immediate_size(op, &code[pc + 1..])).unwrap_or(0) as usize;

        if pc + 1 + immediate_size > code.len() {
            eyre::bail!("incomplete sequence of bytecode");
        }

        steps.push(Instruction { op, pc, immediate: &code[pc + 1..pc + 1 + immediate_size] });

        pc += 1 + immediate_size;
    }

    Ok(steps)
}
