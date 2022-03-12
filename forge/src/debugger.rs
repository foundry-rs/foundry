use ethers::types::{Address, H256};
use revm::{Memory, OpCode};
use std::{borrow::Cow, fmt::Display};

/// An arena of `DebugNode`s
#[derive(Debug, Clone)]
pub struct DebugArena {
    /// The arena of nodes
    pub arena: Vec<DebugNode>,
}

impl Default for DebugArena {
    fn default() -> Self {
        DebugArena { arena: vec![Default::default()] }
    }
}

impl DebugArena {
    /// Pushes a new debug node into the arena
    pub fn push_node(&mut self, entry: usize, mut new_node: DebugNode) {
        match new_node.depth {
            // The entry node, just update it
            0 => {
                self.arena[entry] = new_node;
            }
            // We found the parent node, add the new node as a child
            _ if self.arena[entry].depth == new_node.depth - 1 => {
                new_node.idx = self.arena.len();
                new_node.location = self.arena[entry].children.len();
                self.arena[entry].children.push(new_node.idx);
                self.arena.push(new_node);
            }
            // We haven't found the parent node, go deeper
            _ => self.push_node(
                *self.arena[entry].children.last().expect("Disconnected debug node"),
                new_node,
            ),
        }
    }

    /// Recursively traverses the tree of debug nodes and flattens it into a [Vec] where each
    /// item contains:
    ///
    /// - The address of the contract being executed
    /// - A [Vec] of debug steps along that contract's execution path
    /// - A boolean denoting
    /// Recursively traverses the tree of debug step nodes and flattens it into a
    /// vector where each element contains
    /// 1. the address of the contract being executed
    /// 2. a vector of all the debug steps along that contract's execution path.
    /// 3. Whether the contract was created in this node or not
    ///
    /// This makes it easy to pretty print the execution steps.
    pub fn flatten(&self, entry: usize, flattened: &mut Vec<(Address, Vec<DebugStep>, bool)>) {
        let node = &self.arena[entry];
        flattened.push((node.address, node.steps.clone(), node.creation));
        node.children.iter().for_each(|child| {
            self.flatten(*child, flattened);
        });
    }
}

/// A node in the arena
#[derive(Default, Debug, Clone)]
pub struct DebugNode {
    /// Parent node index in the arena
    pub parent: Option<usize>,
    /// Children node indexes in the arena
    pub children: Vec<usize>,
    /// Location in parent
    pub location: usize,
    /// This node's index in the arena
    pub idx: usize,
    /// Address context
    pub address: Address,
    /// Depth
    pub depth: usize,
    /// The debug steps
    pub steps: Vec<DebugStep>,
    /// Whether the contract was created in this node or not
    pub creation: bool,
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
#[derive(Debug, Clone)]
pub struct DebugStep {
    /// The program counter
    pub pc: usize,
    /// Stack *prior* to running the associated opcode
    pub stack: Vec<H256>,
    /// Memory *prior* to running the associated opcode
    pub memory: Memory,
    /// Opcode to be executed
    pub instruction: Instruction,
    /// Optional bytes that are being pushed onto the stack
    pub push_bytes: Option<Vec<u8>>,
    /// Instruction counter, used to map this instruction to source code
    pub ic: usize,
    /// Cumulative gas usage
    pub total_gas_used: u64,
}

impl Default for DebugStep {
    fn default() -> Self {
        Self {
            pc: 0,
            stack: vec![],
            memory: Memory::new(),
            instruction: Instruction::OpCode(revm::opcode::INVALID),
            push_bytes: None,
            ic: 0,
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

impl Display for DebugStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(push_bytes) = &self.push_bytes {
            write!(
                f,
                "pc: {:?}\nop: {}(0x{})\nstack: {:#?}\nmemory: 0x{}\n\n",
                self.pc,
                self.instruction,
                hex::encode(push_bytes),
                self.stack,
                hex::encode(self.memory.data())
            )
        } else {
            write!(
                f,
                "pc: {:?}\nop: {}\nstack: {:#?}\nmemory: 0x{}\n\n",
                self.pc,
                self.instruction,
                self.stack,
                hex::encode(self.memory.data())
            )
        }
    }
}

/// Forge specific identifiers for cheatcodes
#[derive(Debug, Copy, Clone)]
pub enum CheatOp {
    ROLL,
    WARP,
    FEE,
    STORE,
    LOAD,
    FFI,
    ADDR,
    SIGN,
    PRANK,
    STARTPRANK,
    STOPPRANK,
    DEAL,
    ETCH,
    EXPECTREVERT,
    RECORD,
    ACCESSES,
    EXPECTEMIT,
    MOCKCALL,
    CLEARMOCKEDCALLS,
    EXPECTCALL,
    GETCODE,
    LABEL,
    ASSUME,
}

impl From<CheatOp> for Instruction {
    fn from(cheat: CheatOp) -> Instruction {
        Instruction::Cheatcode(cheat)
    }
}

impl CheatOp {
    /// Gets the name of the cheatcode
    pub const fn name(&self) -> &'static str {
        match self {
            CheatOp::ROLL => "VM_ROLL",
            CheatOp::WARP => "VM_WARP",
            CheatOp::FEE => "VM_FEE",
            CheatOp::STORE => "VM_STORE",
            CheatOp::LOAD => "VM_LOAD",
            CheatOp::FFI => "VM_FFI",
            CheatOp::ADDR => "VM_ADDR",
            CheatOp::SIGN => "VM_SIGN",
            CheatOp::PRANK => "VM_PRANK",
            CheatOp::STARTPRANK => "VM_STARTPRANK",
            CheatOp::STOPPRANK => "VM_STOPPRANK",
            CheatOp::DEAL => "VM_DEAL",
            CheatOp::ETCH => "VM_ETCH",
            CheatOp::EXPECTREVERT => "VM_EXPECTREVERT",
            CheatOp::RECORD => "VM_RECORD",
            CheatOp::ACCESSES => "VM_ACCESSES",
            CheatOp::EXPECTEMIT => "VM_EXPECTEMIT",
            CheatOp::MOCKCALL => "VM_MOCKCALL",
            CheatOp::CLEARMOCKEDCALLS => "VM_CLEARMOCKEDCALLS",
            CheatOp::EXPECTCALL => "VM_EXPECTCALL",
            CheatOp::GETCODE => "VM_GETCODE",
            CheatOp::LABEL => "VM_LABEL",
            CheatOp::ASSUME => "VM_ASSUME",
        }
    }
}

impl Default for CheatOp {
    fn default() -> Self {
        CheatOp::ROLL
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Instruction {
    OpCode(u8),
    Cheatcode(CheatOp),
}

impl From<u8> for Instruction {
    fn from(op: u8) -> Instruction {
        Instruction::OpCode(op)
    }
}

impl Instruction {
    /// The number of bytes being pushed by this instruction, if it is a push.
    pub fn push_size(self) -> Option<u8> {
        match self {
            Instruction::OpCode(op) => OpCode::is_push(op),
            _ => None,
        }
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Instruction::OpCode(op) => OpCode::try_from_u8(*op).map_or_else(
                || Cow::Owned(format!("UNDEFINED(0x{:02x})", op)),
                |opcode| Cow::Borrowed(opcode.as_str()),
            ),
            Instruction::Cheatcode(cheat) => Cow::Borrowed(cheat.name()),
        };

        write!(f, "{}", name)
    }
}
