use sputnik::{Memory, Opcode};

use ethers::types::{Address, H256};

use std::{borrow::Cow, fmt::Display};

#[derive(Debug, Clone)]
/// An arena of `DebugNode`s
pub struct DebugArena {
    /// The arena of nodes
    pub arena: Vec<DebugNode>,
    /// The entry index, denoting the first node's index in the arena
    pub entry: usize,
}

impl Default for DebugArena {
    fn default() -> Self {
        DebugArena { arena: vec![Default::default()], entry: 0 }
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
            // we found the parent node, add the new node as a child
            _ if self.arena[entry].depth == new_node.depth - 1 => {
                new_node.idx = self.arena.len();
                new_node.location = self.arena[entry].children.len();
                self.arena[entry].children.push(new_node.idx);
                self.arena.push(new_node);
            }
            // we haven't found the parent node, go deeper
            _ => self.push_node(
                *self.arena[entry].children.last().expect("Disconnected debug node"),
                new_node,
            ),
        }
    }

    /// Recursively traverses the tree of debug step nodes and flattens it into a
    /// vector where each element contains
    /// 1. the address of the contract being executed
    /// 2. a vector of all the debug steps along that contract's execution path.
    ///  
    /// This then makes it easy to pretty print the execution steps.
    pub fn flatten(&self, entry: usize, flattened: &mut Vec<(Address, Vec<DebugStep>, bool)>) {
        let node = &self.arena[entry];
        flattened.push((node.address, node.steps.clone(), node.creation));
        node.children.iter().for_each(|child| {
            self.flatten(*child, flattened);
        });
    }
}

#[derive(Default, Debug, Clone)]
/// A node in the arena
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
    /// Contract Creation
    pub creation: bool,
}

impl DebugNode {
    pub fn new(address: Address, depth: usize, steps: Vec<DebugStep>) -> Self {
        Self { address, depth, steps, ..Default::default() }
    }
}

/// A `DebugStep` is a snapshot of the EVM's runtime state. It holds the current program counter
/// (where in the program you are), the stack and memory (prior to the opcodes execution), any bytes
/// to be pushed onto the stack, and the instruction counter for use with sourcemaps  
#[derive(Debug, Clone)]
pub struct DebugStep {
    /// Program Counter
    pub pc: usize,
    /// Stack *prior* to running this struct's associated opcode
    pub stack: Vec<H256>,
    /// Memory *prior* to running this struct's associated opcode
    pub memory: Memory,
    /// Opcode to be executed
    pub op: OpCode,
    /// Optional bytes that are being pushed onto the stack
    pub push_bytes: Option<Vec<u8>>,
    /// Instruction counter, used for sourcemap mapping to source code
    pub ic: usize,
    /// Cumulative gas usage
    pub total_gas_used: u64,
}

impl Default for DebugStep {
    fn default() -> Self {
        Self {
            pc: 0,
            stack: vec![],
            memory: Memory::new(0),
            op: OpCode(Opcode::INVALID, None),
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
            format!("{}(0x{})", self.op, hex::encode(push_bytes))
        } else {
            self.op.to_string()
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
                self.op,
                hex::encode(push_bytes),
                self.stack,
                hex::encode(self.memory.data())
            )
        } else {
            write!(
                f,
                "pc: {:?}\nop: {}\nstack: {:#?}\nmemory: 0x{}\n\n",
                self.pc,
                self.op,
                self.stack,
                hex::encode(self.memory.data())
            )
        }
    }
}

/// CheatOps are `forge` specific identifiers for cheatcodes since cheatcodes don't touch the evm
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
}

impl From<CheatOp> for OpCode {
    fn from(cheat: CheatOp) -> OpCode {
        OpCode(Opcode(0x0C), Some(cheat))
    }
}

impl CheatOp {
    /// Gets the `CheatOp` as a string for printing purposes
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
        }
    }
}

impl Default for CheatOp {
    fn default() -> Self {
        CheatOp::ROLL
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OpCode(pub Opcode, pub Option<CheatOp>);

impl From<Opcode> for OpCode {
    fn from(op: Opcode) -> OpCode {
        OpCode(op, None)
    }
}

impl OpCode {
    /// Gets the name of the opcode as a string
    pub const fn name(&self) -> &'static str {
        match self.0 {
            Opcode::STOP => "STOP",
            Opcode::ADD => "ADD",
            Opcode::MUL => "MUL",
            Opcode::SUB => "SUB",
            Opcode::DIV => "DIV",
            Opcode::SDIV => "SDIV",
            Opcode::MOD => "MOD",
            Opcode::SMOD => "SMOD",
            Opcode::ADDMOD => "ADDMOD",
            Opcode::MULMOD => "MULMOD",
            Opcode::EXP => "EXP",
            Opcode::SIGNEXTEND => "SIGNEXTEND",
            Opcode::LT => "LT",
            Opcode::GT => "GT",
            Opcode::SLT => "SLT",
            Opcode::SGT => "SGT",
            Opcode::EQ => "EQ",
            Opcode::ISZERO => "ISZERO",
            Opcode::AND => "AND",
            Opcode::OR => "OR",
            Opcode::XOR => "XOR",
            Opcode::NOT => "NOT",
            Opcode::BYTE => "BYTE",
            Opcode::SHL => "SHL",
            Opcode::SHR => "SHR",
            Opcode::SAR => "SAR",
            Opcode::SHA3 => "KECCAK256",
            Opcode::ADDRESS => "ADDRESS",
            Opcode::BALANCE => "BALANCE",
            Opcode::ORIGIN => "ORIGIN",
            Opcode::CALLER => "CALLER",
            Opcode::CALLVALUE => "CALLVALUE",
            Opcode::CALLDATALOAD => "CALLDATALOAD",
            Opcode::CALLDATASIZE => "CALLDATASIZE",
            Opcode::CALLDATACOPY => "CALLDATACOPY",
            Opcode::CODESIZE => "CODESIZE",
            Opcode::CODECOPY => "CODECOPY",
            Opcode::GASPRICE => "GASPRICE",
            Opcode::EXTCODESIZE => "EXTCODESIZE",
            Opcode::EXTCODECOPY => "EXTCODECOPY",
            Opcode::RETURNDATASIZE => "RETURNDATASIZE",
            Opcode::RETURNDATACOPY => "RETURNDATACOPY",
            Opcode::EXTCODEHASH => "EXTCODEHASH",
            Opcode::BLOCKHASH => "BLOCKHASH",
            Opcode::COINBASE => "COINBASE",
            Opcode::TIMESTAMP => "TIMESTAMP",
            Opcode::NUMBER => "NUMBER",
            Opcode::DIFFICULTY => "DIFFICULTY",
            Opcode::GASLIMIT => "GASLIMIT",
            Opcode::CHAINID => "CHAINID",
            Opcode::SELFBALANCE => "SELFBALANCE",
            Opcode::BASEFEE => "BASEFEE",
            Opcode::POP => "POP",
            Opcode::MLOAD => "MLOAD",
            Opcode::MSTORE => "MSTORE",
            Opcode::MSTORE8 => "MSTORE8",
            Opcode::SLOAD => "SLOAD",
            Opcode::SSTORE => "SSTORE",
            Opcode::JUMP => "JUMP",
            Opcode::JUMPI => "JUMPI",
            Opcode::PC => "PC",
            Opcode::MSIZE => "MSIZE",
            Opcode::GAS => "GAS",
            Opcode::JUMPDEST => "JUMPDEST",
            Opcode::PUSH1 => "PUSH1",
            Opcode::PUSH2 => "PUSH2",
            Opcode::PUSH3 => "PUSH3",
            Opcode::PUSH4 => "PUSH4",
            Opcode::PUSH5 => "PUSH5",
            Opcode::PUSH6 => "PUSH6",
            Opcode::PUSH7 => "PUSH7",
            Opcode::PUSH8 => "PUSH8",
            Opcode::PUSH9 => "PUSH9",
            Opcode::PUSH10 => "PUSH10",
            Opcode::PUSH11 => "PUSH11",
            Opcode::PUSH12 => "PUSH12",
            Opcode::PUSH13 => "PUSH13",
            Opcode::PUSH14 => "PUSH14",
            Opcode::PUSH15 => "PUSH15",
            Opcode::PUSH16 => "PUSH16",
            Opcode::PUSH17 => "PUSH17",
            Opcode::PUSH18 => "PUSH18",
            Opcode::PUSH19 => "PUSH19",
            Opcode::PUSH20 => "PUSH20",
            Opcode::PUSH21 => "PUSH21",
            Opcode::PUSH22 => "PUSH22",
            Opcode::PUSH23 => "PUSH23",
            Opcode::PUSH24 => "PUSH24",
            Opcode::PUSH25 => "PUSH25",
            Opcode::PUSH26 => "PUSH26",
            Opcode::PUSH27 => "PUSH27",
            Opcode::PUSH28 => "PUSH28",
            Opcode::PUSH29 => "PUSH29",
            Opcode::PUSH30 => "PUSH30",
            Opcode::PUSH31 => "PUSH31",
            Opcode::PUSH32 => "PUSH32",
            Opcode::DUP1 => "DUP1",
            Opcode::DUP2 => "DUP2",
            Opcode::DUP3 => "DUP3",
            Opcode::DUP4 => "DUP4",
            Opcode::DUP5 => "DUP5",
            Opcode::DUP6 => "DUP6",
            Opcode::DUP7 => "DUP7",
            Opcode::DUP8 => "DUP8",
            Opcode::DUP9 => "DUP9",
            Opcode::DUP10 => "DUP10",
            Opcode::DUP11 => "DUP11",
            Opcode::DUP12 => "DUP12",
            Opcode::DUP13 => "DUP13",
            Opcode::DUP14 => "DUP14",
            Opcode::DUP15 => "DUP15",
            Opcode::DUP16 => "DUP16",
            Opcode::SWAP1 => "SWAP1",
            Opcode::SWAP2 => "SWAP2",
            Opcode::SWAP3 => "SWAP3",
            Opcode::SWAP4 => "SWAP4",
            Opcode::SWAP5 => "SWAP5",
            Opcode::SWAP6 => "SWAP6",
            Opcode::SWAP7 => "SWAP7",
            Opcode::SWAP8 => "SWAP8",
            Opcode::SWAP9 => "SWAP9",
            Opcode::SWAP10 => "SWAP10",
            Opcode::SWAP11 => "SWAP11",
            Opcode::SWAP12 => "SWAP12",
            Opcode::SWAP13 => "SWAP13",
            Opcode::SWAP14 => "SWAP14",
            Opcode::SWAP15 => "SWAP15",
            Opcode::SWAP16 => "SWAP16",
            Opcode::LOG0 => "LOG0",
            Opcode::LOG1 => "LOG1",
            Opcode::LOG2 => "LOG2",
            Opcode::LOG3 => "LOG3",
            Opcode::LOG4 => "LOG4",
            Opcode::CREATE => "CREATE",
            Opcode::CALL => "CALL",
            Opcode::CALLCODE => "CALLCODE",
            Opcode::RETURN => "RETURN",
            Opcode::DELEGATECALL => "DELEGATECALL",
            Opcode::CREATE2 => "CREATE2",
            Opcode::STATICCALL => "STATICCALL",
            Opcode::REVERT => "REVERT",
            Opcode::INVALID => "INVALID",
            Opcode::SUICIDE => "SELFDESTRUCT",
            _ => {
                if let Some(cheat) = self.1 {
                    cheat.name()
                } else {
                    "UNDEFINED"
                }
            }
        }
    }

    /// Optionally return the push size of the opcode if it is a push
    pub fn push_size(self) -> Option<u8> {
        self.0.is_push()
    }
}

impl Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.name();

        let n = if name == "UNDEFINED" {
            Cow::Owned(format!("UNDEFINED(0x{:02x})", self.0 .0))
        } else {
            Cow::Borrowed(name)
        };
        write!(f, "{}", n)
    }
}
