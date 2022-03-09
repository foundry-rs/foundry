use crate::executor::CHEATCODE_ADDRESS;
use ansi_term::Colour;
use ethers::{
    abi::{Abi, Address, Event, Function, RawLog, Token},
    types::{H160, H256, U256},
};
use foundry_utils::format_token;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An arena of `CallTraceNode`s
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallTraceArena {
    /// The arena of nodes
    pub arena: Vec<CallTraceNode>,
}

impl Default for CallTraceArena {
    fn default() -> Self {
        CallTraceArena { arena: vec![Default::default()] }
    }
}

// Gets pretty print strings for tokens
pub fn format_labeled_token(param: &Token, exec_info: &ExecutionInfo<'_>) -> String {
    match param {
        Token::Address(addr) => {
            if let Some(label) = exec_info.labeled_addrs.get(addr) {
                format!("{} [{:?}]", label, addr)
            } else {
                format_token(param)
            }
        }
        _ => format_token(param),
    }
}

/// Function output type
pub enum Output {
    /// Decoded vec of tokens
    Token(Vec<ethers::abi::Token>),
    /// Not decoded raw bytes
    Raw(Vec<u8>),
}

/// A struct with all information about execution
pub struct ExecutionInfo<'a> {
    pub contracts: &'a BTreeMap<String, (Abi, Vec<u8>)>,
    pub labeled_addrs: &'a BTreeMap<H160, String>,
    pub funcs: &'a BTreeMap<[u8; 4], Function>,
    pub events: &'a BTreeMap<H256, Event>,
    pub errors: &'a Abi,
}

impl<'a> ExecutionInfo<'a> {
    pub fn new(
        contracts: &'a BTreeMap<String, (Abi, Vec<u8>)>,
        labeled_addrs: &'a BTreeMap<H160, String>,
        funcs: &'a BTreeMap<[u8; 4], Function>,
        events: &'a BTreeMap<H256, Event>,
        errors: &'a Abi,
    ) -> Self {
        Self { contracts, labeled_addrs, funcs, events, errors }
    }
}

impl Output {
    pub fn construct_string<'a>(
        self,
        exec_info: &ExecutionInfo<'a>,
        color: Colour,
        left: &str,
    ) -> String {
        let formatted = match self {
            Output::Token(token) => {
                let strings = token
                    .iter()
                    .map(|token| format_labeled_token(token, exec_info))
                    .collect::<Vec<_>>()
                    .join(", ");
                if strings.is_empty() {
                    "()".to_string()
                } else {
                    strings
                }
            }
            Output::Raw(bytes) => {
                if bytes.is_empty() {
                    "()".to_string()
                } else {
                    "0x".to_string() + &hex::encode(&bytes)
                }
            }
        };

        format!(
            "\n{}  └─ {} {}",
            left.replace("├─", "│").replace("└─", "  "),
            color.paint("←"),
            formatted
        )
    }
}

impl CallTraceArena {
    /// Pushes a new trace into the arena, returning the trace ID
    pub fn push_trace(&mut self, entry: usize, mut new_trace: CallTrace) -> usize {
        match new_trace.depth {
            // The entry node, just update it
            0 => {
                let idx = new_trace.idx;
                self.update(new_trace);
                idx
            }
            // We found the parent node, add the new trace as a child
            _ if self.arena[entry].trace.depth == new_trace.depth - 1 => {
                let idx = self.arena.len();
                new_trace.idx = idx;
                new_trace.location = self.arena[entry].children.len();
                self.arena[entry].ordering.push(LogCallOrder::Call(new_trace.location));
                let node = CallTraceNode {
                    parent: Some(entry),
                    idx,
                    trace: new_trace,
                    ..Default::default()
                };
                self.arena.push(node);
                self.arena[entry].children.push(idx);
                idx
            }
            // We haven't found the parent node, go deeper
            _ => self.push_trace(
                *self.arena[entry].children.last().expect("Disconnected trace"),
                new_trace,
            ),
        }
    }

    /// Updates the values in the calltrace held by the arena based on the passed in trace
    pub fn update(&mut self, trace: CallTrace) {
        let node = &mut self.arena[trace.idx];
        node.trace.update(trace);
    }

    /*/// Updates `identified_contracts` for future use so that after an `evm.reset_state()`, we
    /// already know which contract corresponds to which address.
    ///
    /// `idx` is the call arena index to start at. Generally this will be 0, but if you want to
    /// update a subset of the tree, you can pass in a different index
    ///
    /// `contracts` are the known contracts of (name => (abi, runtime_code)). It is used to identify
    /// a deployed contract.
    ///
    /// `identified_contracts` are the identified contract addresses built up from comparing
    /// deployed contracts against `contracts`
    ///
    /// `evm` is the evm that we used so that we can grab deployed code if needed. A lot of times,
    /// the evm state is reset so we wont have any code but it can be useful if we want to
    /// pretty print right after a test.
    pub fn update_identified<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        idx: usize,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
    ) {
        let trace = &self.arena[idx].trace;

        #[cfg(feature = "sputnik")]
        identified_contracts.insert(*CHEATCODE_ADDRESS, ("VM".to_string(), HEVM_ABI.clone()));

        let res = identified_contracts.get(&trace.addr);
        if res.is_none() {
            let code = if trace.created { trace.output.clone() } else { evm.code(trace.addr) };
            if let Some((name, (abi, _code))) = contracts
                .iter()
                .find(|(_key, (_abi, known_code))| diff_score(known_code, &code) < 0.10)
            {
                identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
            }
        }

        // update all children nodes
        self.update_children(idx, contracts, identified_contracts, evm);
    }

    /// Updates all children nodes by recursing into `update_identified`
    pub fn update_children<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        idx: usize,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
    ) {
        let children_idxs = &self.arena[idx].children;
        children_idxs.iter().for_each(|child_idx| {
            self.update_identified(*child_idx, contracts, identified_contracts, evm);
        });
    }*/

    /// Construct a CallTraceArena trace string
    ///
    /// `idx` is the call arena index to start at. Generally this will be 0, but if you want to
    /// print a subset of the tree, you can pass in a different index
    ///
    /// `contracts` are the known contracts of (name => (abi, runtime_code)). It is used to identify
    /// a deployed contract.
    ///
    /// `evm` is the evm that we used so that we can grab deployed code if needed. A lot of times,
    /// the evm state is reset so we wont have any code but it can be useful if we want to
    /// pretty print right after a test.
    ///
    /// For a user, `left` input should generally be `""`. Left is used recursively
    /// to build the tree print out structure and is built up as we recurse down the tree.
    pub fn construct_trace_string<'a>(
        &self,
        idx: usize,
        exec_info: &mut ExecutionInfo<'a>,
        left: &str,
    ) -> String {
        let trace = &self.arena[idx].trace;

        // color the trace function call & output by success
        let color = if trace.address == *CHEATCODE_ADDRESS {
            Colour::Blue
        } else if trace.success {
            Colour::Green
        } else {
            Colour::Red
        };

        if trace.created {
            String::from_iter([
                format!(
                    "\n{}{} {}@{}",
                    left,
                    Colour::Yellow.paint("→ new"),
                    trace.label.as_ref().unwrap_or(&"<Unknown>".to_string()),
                    trace.address
                ),
                self.construct_children_and_logs(idx, exec_info, left),
                format!(
                    "\n{}  └─ {} {} bytes of code",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    trace.output.len()
                ),
            ])
        } else {
            let (call, ret) = trace.construct_func_call(exec_info, color, left);
            String::from_iter([call, self.construct_children_and_logs(idx, exec_info, left), ret])
        }
    }

    /// Prints child calls and logs in order
    pub fn construct_children_and_logs<'a>(
        &self,
        node_idx: usize,
        exec_info: &mut ExecutionInfo<'a>,
        left: &str,
    ) -> String {
        // Ordering stores a vec of `LogCallOrder` which is populated based on if
        // a log or a call was called first. This makes it such that we always print
        // logs and calls in the correct order
        self.arena[node_idx]
            .ordering
            .iter()
            .map(|ordering| match ordering {
                LogCallOrder::Log(index) => {
                    self.arena[node_idx].construct_log(exec_info, *index, exec_info.events, left)
                }
                LogCallOrder::Call(index) => self.construct_trace_string(
                    self.arena[node_idx].children[*index],
                    exec_info,
                    &(left.replace("├─", "│").replace("└─", "  ") + "  ├─ "),
                ),
            })
            .collect()
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
/// A node in the arena
pub struct CallTraceNode {
    /// Parent node index in the arena
    pub parent: Option<usize>,
    /// Children node indexes in the arena
    pub children: Vec<usize>,
    /// This node's index in the arena
    pub idx: usize,
    /// The call trace
    pub trace: CallTrace,
    /// Logs
    #[serde(skip)]
    pub logs: Vec<RawLog>,
    /// Ordering of child calls and logs
    pub ordering: Vec<LogCallOrder>,
}

impl CallTraceNode {
    /// Prints a log at a particular index, optionally decoding if abi is provided
    pub fn construct_log<'a, 'b>(
        &self,
        exec_info: &'b ExecutionInfo<'a>,
        index: usize,
        events: &BTreeMap<H256, Event>,
        left: &str,
    ) -> String {
        let log = &self.logs[index];
        let right = "  ├─ ";

        if let Some(event) = events.get(&log.topics[0]) {
            if let Ok(parsed) = event.parse_log(log.clone()) {
                let params = parsed.params;
                let strings = params
                    .into_iter()
                    .map(|param| {
                        format!("{}: {}", param.name, format_labeled_token(&param.value, exec_info))
                    })
                    .collect::<Vec<String>>()
                    .join(", ");

                return format!(
                    "\n{}emit {}({})",
                    left.replace("├─", "│") + right,
                    Colour::Cyan.paint(event.name.clone()),
                    strings
                )
            }
        }

        // We couldn't decode the log
        let formatted: String = log
            .topics
            .iter()
            .enumerate()
            .map(|(i, topic)| {
                let right = if i == log.topics.len() - 1 && log.data.is_empty() {
                    "  └─ "
                } else {
                    "  ├─"
                };
                format!(
                    "\n{}{}topic {}: {}",
                    if i == 0 {
                        left.replace("├─", "│") + right
                    } else {
                        left.replace("├─", "│") + "  │ "
                    },
                    if i == 0 { " emit " } else { "      " },
                    i,
                    Colour::Cyan.paint(format!("0x{}", hex::encode(&topic)))
                )
            })
            .collect();

        format!(
            "{}\n{}        data: {}",
            formatted,
            left.replace("├─", "│").replace("└─", "  ") + "  │  ",
            Colour::Cyan.paint(format!("0x{}", hex::encode(&log.data)))
        )
    }
}

/// Ordering enum for calls and logs
///
/// i.e. if Call 0 occurs before Log 0, it will be pushed into the `CallTraceNode`'s ordering before
/// the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogCallOrder {
    Log(usize),
    Call(usize),
}

/// A trace of a call.
///
/// Traces come in two forms: raw and decoded.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CallTrace {
    /// The depth of the call
    pub depth: usize,
    /// TODO: Docs
    pub location: usize,
    /// The ID of this trace
    pub idx: usize,
    /// Whether the call was successful
    pub success: bool,
    /// The label for the destination address, if any
    pub label: Option<String>,
    /// The destination address of the call
    pub address: Address,
    /// Whether the call was a contract creation or not
    pub created: bool,
    /// The value tranferred in the call
    pub value: U256,
    /// The called function if we could determine it
    pub function: Option<Function>,
    /// The calldata for the call, or the init code for contract creations
    pub data: Vec<u8>,
    /// The return data of the call if this was not a contract creation, otherwise it is the
    /// runtime bytecode of the created contract
    pub output: Vec<u8>,
    /// The gas cost of the call
    pub gas_cost: u64,
}

impl CallTrace {
    /// Updates a trace given another trace
    fn update(&mut self, new_trace: Self) {
        self.success = new_trace.success;
        self.address = new_trace.address;
        self.gas_cost = new_trace.gas_cost;
        self.output = new_trace.output;
        self.data = new_trace.data;
        self.address = new_trace.address;
    }

    /// Prints function call, returning the decoded or raw output
    pub fn construct_func_call<'a>(
        &self,
        exec_info: &mut ExecutionInfo<'a>,
        color: Colour,
        left: &str,
    ) -> (String, String) {
        let (function_name, inputs, outputs) = if let Some(func) = &self.function {
            // Attempt to decode function inputs
            let mut inputs = if !self.data[4..].is_empty() {
                func.decode_input(&self.data[4..])
                    .expect("could not decode inputs")
                    .iter()
                    .map(|token| format_labeled_token(token, exec_info))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "".to_string()
            };

            // Better decoding for inputs to `expectRevert`
            if self.address == *CHEATCODE_ADDRESS && func.name == "expectRevert" {
                if let Ok(decoded) =
                    foundry_utils::decode_revert(&self.data, Some(exec_info.errors))
                {
                    inputs = decoded;
                }
            }

            // Attempt to decode function outputs/reverts
            let outputs = if self.output.is_empty() {
                Output::Raw(vec![])
            } else if self.success {
                if let Ok(tokens) = func.decode_output(&self.output[..]) {
                    Output::Token(tokens)
                } else {
                    Output::Raw(self.output.clone())
                }
            } else {
                if let Ok(decoded_error) =
                    foundry_utils::decode_revert(&self.output[..], Some(exec_info.errors))
                {
                    Output::Token(vec![ethers::abi::Token::String(decoded_error)])
                } else {
                    Output::Raw(self.output.clone())
                }
            };

            (func.name.clone(), inputs, outputs)
        } else {
            // Attempt to decode reverts
            let outputs = if !self.success {
                if let Ok(decoded_error) =
                    foundry_utils::decode_revert(&self.output[..], Some(exec_info.errors))
                {
                    Output::Token(vec![ethers::abi::Token::String(decoded_error)])
                } else {
                    Output::Raw(self.output.clone())
                }
            } else {
                Output::Raw(self.output.clone())
            };

            if self.data.len() < 4 {
                ("fallback".to_string(), String::new(), outputs)
            } else {
                (hex::encode(&self.data[0..4]), hex::encode(&self.data[4..]), outputs)
            }
        };
        let transfer = if !self.value.is_zero() {
            format!("{{value: {}}}", self.value)
        } else {
            "".to_string()
        };

        (
            format!(
                "\n{}[{}] {}::{}{}({})",
                left,
                self.gas_cost,
                color.paint(self.label.as_ref().unwrap_or(&self.address.to_string())),
                color.paint(function_name),
                transfer,
                inputs,
            ),
            outputs.construct_string(exec_info, color, left),
        )
    }
}

// very simple fuzzy matching to account for immutables. Will fail for small contracts that are
// basically all immutable vars
/*fn diff_score(bytecode1: &[u8], bytecode2: &[u8]) -> f64 {
    let cutoff_len = usize::min(bytecode1.len(), bytecode2.len());
    let b1 = &bytecode1[..cutoff_len];
    let b2 = &bytecode2[..cutoff_len];
    if cutoff_len == 0 {
        return 1.0
    }

    let mut diff_chars = 0;
    for i in 0..cutoff_len {
        if b1[i] != b2[i] {
            diff_chars += 1;
        }
    }

    // println!("diff_score {}", diff_chars as f64 / cutoff_len as f64);
    diff_chars as f64 / cutoff_len as f64
}*/
