use crate::{abi::HEVM_ABI, CallKind};
use alloy_primitives::{Address, U256};
use revm::interpreter::{Memory, OpCode};
use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// An arena of [DebugNode]s
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct DebugArena {
    /// The arena of nodes
    pub arena: Vec<DebugNode>,
}

impl DebugArena {
    /// Pushes a new debug node into the arena
    pub fn push_node(&mut self, mut new_node: DebugNode) -> usize {
        fn recursively_push(
            arena: &mut Vec<DebugNode>,
            entry: usize,
            mut new_node: DebugNode,
        ) -> usize {
            match new_node.depth {
                // We found the parent node, add the new node as a child
                _ if arena[entry].depth == new_node.depth - 1 => {
                    let id = arena.len();
                    new_node.location = arena[entry].children.len();
                    new_node.parent = Some(entry);
                    arena[entry].children.push(id);
                    arena.push(new_node);
                    id
                }
                // We haven't found the parent node, go deeper
                _ => {
                    let child = *arena[entry].children.last().expect("Disconnected debug node");
                    recursively_push(arena, child, new_node)
                }
            }
        }

        if self.arena.is_empty() {
            // This is the initial node at depth 0, so we just insert it.
            self.arena.push(new_node);
            0
        } else if new_node.depth == 0 {
            // This is another node at depth 0, for example instructions between calls. We insert
            // it as a child of the original root node.
            let id = self.arena.len();
            new_node.location = self.arena[0].children.len();
            new_node.parent = Some(0);
            self.arena[0].children.push(id);
            self.arena.push(new_node);
            id
        } else {
            // We try to find the parent of this node recursively
            recursively_push(&mut self.arena, 0, new_node)
        }
    }

    /// Recursively traverses the tree of debug nodes and flattens it into a [Vec] where each
    /// item contains:
    ///
    /// - The address of the contract being executed
    /// - A [Vec] of debug steps along that contract's execution path
    /// - An enum denoting the type of call this is
    ///
    /// This makes it easy to pretty print the execution steps.
    pub fn flatten(&self, entry: usize) -> Vec<(Address, Vec<DebugStep>, CallKind)> {
        let node = &self.arena[entry];

        let mut flattened = vec![];
        if !node.steps.is_empty() {
            flattened.push((node.address, node.steps.clone(), node.kind));
        }
        flattened.extend(node.children.iter().flat_map(|child| self.flatten(*child)));

        flattened
    }
}

/// A node in the arena
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct DebugNode {
    /// Parent node index in the arena
    pub parent: Option<usize>,
    /// Children node indexes in the arena
    pub children: Vec<usize>,
    /// Location in parent
    pub location: usize,
    /// Execution context.
    ///
    /// Note that this is the address of the *code*, not necessarily the address of the storage.
    pub address: Address,
    /// The kind of call this is
    pub kind: CallKind,
    /// Depth
    pub depth: usize,
    /// The debug steps
    pub steps: Vec<DebugStep>,
}

impl DebugNode {
    pub fn new(address: Address, depth: usize, steps: Vec<DebugStep>) -> Self {
        Self { address, depth, steps, ..Default::default() }
    }
}

/// A `DebugStep` is a snapshot of the EVM's runtime state.
///
/// It holds the current program counter (where in the program you are),
/// the stack and memory (prior to the opcodes execution), any bytes to be
/// pushed onto the stack, and the instruction counter for use with sourcemap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugStep {
    /// Stack *prior* to running the associated opcode
    pub stack: Vec<U256>,
    /// Memory *prior* to running the associated opcode
    pub memory: Memory,
    /// Opcode to be executed
    pub instruction: Instruction,
    /// Optional bytes that are being pushed onto the stack
    pub push_bytes: Option<Vec<u8>>,
    /// The program counter at this step.
    ///
    /// Note: To map this step onto source code using a source map, you must convert the program
    /// counter to an instruction counter.
    pub pc: usize,
    /// Cumulative gas usage
    pub total_gas_used: u64,
}

impl Default for DebugStep {
    fn default() -> Self {
        Self {
            stack: vec![],
            memory: Memory::new(),
            instruction: Instruction::OpCode(revm::interpreter::opcode::INVALID),
            push_bytes: None,
            pc: 0,
            total_gas_used: 0,
        }
    }
}

impl DebugStep {
    /// Pretty print the step's opcode
    pub fn pretty_opcode(&self) -> String {
        if let Some(push_bytes) = &self.push_bytes {
            format!("{}(0x{})", self.instruction, hex::encode(push_bytes))
        } else {
            self.instruction.to_string()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Instruction {
    OpCode(u8),
    Cheatcode([u8; 4]),
}

impl From<u8> for Instruction {
    fn from(op: u8) -> Instruction {
        Instruction::OpCode(op)
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Instruction::OpCode(op) => write!(
                f,
                "{}",
                OpCode::try_from_u8(*op).map_or_else(
                    || format!("UNDEFINED(0x{op:02x})"),
                    |opcode| opcode.as_str().to_string(),
                )
            ),
            Instruction::Cheatcode(cheat) => write!(
                f,
                "VM_{}",
                &*HEVM_ABI
                    .functions()
                    .find(|func| func.short_signature() == *cheat)
                    .expect("unknown cheatcode found in debugger")
                    .name
                    .to_uppercase()
            ),
        }
    }
}
