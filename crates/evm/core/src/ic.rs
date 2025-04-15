use alloy_primitives::map::rustc_hash::FxHashMap;
use eyre::Result;
use revm::interpreter::{
    opcode::{PUSH0, PUSH1, PUSH32},
    OpCode,
};
use revm_inspectors::opcode::immediate_size;
use serde::Serialize;

/// Maps from program counter to instruction counter.
///
/// Inverse of [`IcPcMap`].
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct PcIcMap {
    pub inner: FxHashMap<u32, u32>,
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
    pub fn get(&self, pc: u32) -> Option<u32> {
        self.inner.get(&pc).copied()
    }
}

/// Map from instruction counter to program counter.
///
/// Inverse of [`PcIcMap`].
pub struct IcPcMap {
    pub inner: FxHashMap<u32, u32>,
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
    pub fn get(&self, ic: u32) -> Option<u32> {
        self.inner.get(&ic).copied()
    }
}

fn make_map<const PC_FIRST: bool>(code: &[u8]) -> FxHashMap<u32, u32> {
    assert!(code.len() <= u32::MAX as usize, "bytecode is too big");

    let mut map = FxHashMap::with_capacity_and_hasher(code.len(), Default::default());

    let mut pc = 0usize;
    let mut cumulative_push_size = 0usize;
    while pc < code.len() {
        let ic = pc - cumulative_push_size;
        if PC_FIRST {
            map.insert(pc as u32, ic as u32);
        } else {
            map.insert(ic as u32, pc as u32);
        }

        if (PUSH1..=PUSH32).contains(&code[pc]) {
            // Skip the push bytes.
            let push_size = (code[pc] - PUSH0) as usize;
            pc += push_size;
            cumulative_push_size += push_size;
        }

        pc += 1;
    }

    map.shrink_to_fit();

    map
}

/// Represents a single instruction consisting of the opcode and its immediate data.
pub struct Instruction<'a> {
    /// OpCode, if it could be decoded.
    pub op: Option<OpCode>,
    /// Immediate data following the opcode.
    pub immediate: &'a [u8],
    /// Program counter of the opcode.
    pub pc: u32,
}

/// Decodes raw opcode bytes into [`Instruction`]s.
pub fn decode_instructions(code: &[u8]) -> Result<Vec<Instruction<'_>>> {
    assert!(code.len() <= u32::MAX as usize, "bytecode is too big");

    let mut pc = 0usize;
    let mut steps = Vec::new();

    while pc < code.len() {
        let op = OpCode::new(code[pc]);
        pc += 1;
        let immediate_size = op.map(|op| immediate_size(op, &code[pc..])).unwrap_or(0) as usize;

        if pc + immediate_size > code.len() {
            eyre::bail!("incomplete sequence of bytecode");
        }

        steps.push(Instruction { op, pc: pc as u32, immediate: &code[pc..pc + immediate_size] });

        pc += immediate_size;
    }

    Ok(steps)
}
