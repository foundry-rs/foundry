use ethers::{
    abi::{Abi, FunctionExt, RawLog},
    types::{H160, U256},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use ansi_term::Colour;

#[cfg(feature = "sputnik")]
use crate::sputnik::cheatcodes::{cheatcode_handler::CHEATCODE_ADDRESS, HEVM_ABI};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// An arena of `CallTraceNode`s
pub struct CallTraceArena {
    /// The arena of nodes
    pub arena: Vec<CallTraceNode>,
    /// The entry index, denoting the first node's index in the arena
    pub entry: usize,
}

impl Default for CallTraceArena {
    fn default() -> Self {
        CallTraceArena { arena: vec![Default::default()], entry: 0 }
    }
}

/// Function output type
pub enum Output {
    /// Decoded vec of tokens
    Token(Vec<ethers::abi::Token>),
    /// Not decoded raw bytes
    Raw(Vec<u8>),
}

impl Output {
    /// Prints the output of a function call
    pub fn print(self, color: Colour, left: &str) {
        match self {
            Output::Token(token) => {
                let strings =
                    token.into_iter().map(format_token).collect::<Vec<String>>().join(", ");
                println!(
                    "{}  └─ {} {}",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    if strings.is_empty() { "()" } else { &*strings }
                );
            }
            Output::Raw(bytes) => {
                println!(
                    "{}  └─ {} {}",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    if bytes.is_empty() {
                        "()".to_string()
                    } else {
                        "0x".to_string() + &hex::encode(&bytes)
                    }
                );
            }
        }
    }
}

impl CallTraceArena {
    /// Pushes a new trace into the arena, returning the trace that was passed in with updated
    /// values
    pub fn push_trace(&mut self, entry: usize, mut new_trace: &mut CallTrace) {
        match new_trace.depth {
            // The entry node, just update it
            0 => {
                self.update(new_trace.clone());
            }
            // we found the parent node, add the new trace as a child
            _ if self.arena[entry].trace.depth == new_trace.depth - 1 => {
                new_trace.idx = self.arena.len();
                new_trace.location = self.arena[entry].children.len();
                self.arena[entry].ordering.push(LogCallOrder::Call(new_trace.location));
                let node = CallTraceNode {
                    parent: Some(entry),
                    idx: self.arena.len(),
                    trace: new_trace.clone(),
                    ..Default::default()
                };
                self.arena.push(node);
                self.arena[entry].children.push(new_trace.idx);
            }
            // we haven't found the parent node, go deeper
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

    /// Updates `identified_contracts` for future use so that after an `evm.reset_state()`, we
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
    }

    /// Pretty print a CallTraceArena
    ///
    /// `idx` is the call arena index to start at. Generally this will be 0, but if you want to
    /// print a subset of the tree, you can pass in a different index
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
    ///
    /// For a user, `left` input should generally be `""`. Left is used recursively
    /// to build the tree print out structure and is built up as we recurse down the tree.
    pub fn pretty_print<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        idx: usize,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
        left: &str,
    ) {
        let trace = &self.arena[idx].trace;

        #[cfg(feature = "sputnik")]
        identified_contracts.insert(*CHEATCODE_ADDRESS, ("VM".to_string(), HEVM_ABI.clone()));

        #[cfg(feature = "sputnik")]
        // color the trace function call & output by success
        let color = if trace.addr == *CHEATCODE_ADDRESS {
            Colour::Blue
        } else if trace.success {
            Colour::Green
        } else {
            Colour::Red
        };

        #[cfg(not(feature = "sputnik"))]
        let color = if trace.success { Colour::Green } else { Colour::Red };

        // we have to clone the name and abi because identified_contracts is later borrowed
        // immutably
        let res = if let Some((name, abi)) = identified_contracts.get(&trace.addr) {
            Some((name.clone(), abi.clone()))
        } else {
            None
        };
        if res.is_none() {
            // get the code to compare
            let code = if trace.created { trace.output.clone() } else { evm.code(trace.addr) };
            if let Some((name, (abi, _code))) = contracts
                .iter()
                .find(|(_key, (_abi, known_code))| diff_score(known_code, &code) < 0.10)
            {
                // found matching contract, insert and print
                identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                if trace.created {
                    println!("{}{} {}@{}", left, Colour::Yellow.paint("→ new"), name, trace.addr);
                    self.print_children_and_logs(
                        idx,
                        Some(abi),
                        contracts,
                        identified_contracts,
                        evm,
                        left,
                    );
                    println!(
                        "{}  └─ {} {} bytes of code",
                        left.replace("├─", "│").replace("└─", "  "),
                        color.paint("←"),
                        trace.output.len()
                    );
                } else {
                    // re-enter this function at the current node
                    self.pretty_print(idx, contracts, identified_contracts, evm, left);
                }
            } else if trace.created {
                // we couldn't identify, print the children and logs without the abi
                println!("{}{} <Unknown>@{}", left, Colour::Yellow.paint("→ new"), trace.addr);
                self.print_children_and_logs(idx, None, contracts, identified_contracts, evm, left);
                println!(
                    "{}  └─ {} {} bytes of code",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    trace.output.len()
                );
            } else {
                let output = trace.print_func_call(None, None, color, left);
                self.print_children_and_logs(idx, None, contracts, identified_contracts, evm, left);
                output.print(color, left);
            }
        } else if let Some((name, abi)) = res {
            if trace.created {
                println!("{}{} {}@{}", left, Colour::Yellow.paint("→ new"), name, trace.addr);
                self.print_children_and_logs(
                    idx,
                    Some(&abi),
                    contracts,
                    identified_contracts,
                    evm,
                    left,
                );
                println!(
                    "{}  └─ {} {} bytes of code",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    trace.output.len()
                );
            } else {
                let output = trace.print_func_call(Some(&abi), Some(&name), color, left);
                self.print_children_and_logs(
                    idx,
                    Some(&abi),
                    contracts,
                    identified_contracts,
                    evm,
                    left,
                );
                output.print(color, left);
            }
        }
    }

    /// Prints child calls and logs in order
    pub fn print_children_and_logs<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        node_idx: usize,
        abi: Option<&Abi>,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
        left: &str,
    ) {
        // Ordering stores a vec of `LogCallOrder` which is populated based on if
        // a log or a call was called first. This makes it such that we always print
        // logs and calls in the correct order
        self.arena[node_idx].ordering.iter().for_each(|ordering| match ordering {
            LogCallOrder::Log(index) => {
                self.arena[node_idx].print_log(*index, abi, left);
            }
            LogCallOrder::Call(index) => {
                self.pretty_print(
                    self.arena[node_idx].children[*index],
                    contracts,
                    identified_contracts,
                    evm,
                    &(left.replace("├─", "│").replace("└─", "  ") + "  ├─ "),
                );
            }
        });
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
    pub fn print_log(&self, index: usize, abi: Option<&Abi>, left: &str) {
        let log = &self.logs[index];
        let right = "  ├─ ";
        if let Some(abi) = abi {
            for (event_name, overloaded_events) in abi.events.iter() {
                for event in overloaded_events.iter() {
                    if event.signature() == log.topics[0] {
                        let params = event.parse_log(log.clone()).expect("Bad event").params;
                        let strings = params
                            .into_iter()
                            .map(|param| format!("{}: {}", param.name, format_token(param.value)))
                            .collect::<Vec<String>>()
                            .join(", ");
                        println!(
                            "{}emit {}({})",
                            left.replace("├─", "│") + right,
                            Colour::Cyan.paint(event_name),
                            strings
                        );
                        return
                    }
                }
            }
        }
        // we didnt decode the log, print it as an unknown log
        for (i, topic) in log.topics.iter().enumerate() {
            let right = if i == log.topics.len() - 1 && log.data.is_empty() {
                "  └─ "
            } else {
                "  ├─"
            };
            println!(
                "{}{}topic {}: {}",
                if i == 0 {
                    left.replace("├─", "│") + right
                } else {
                    left.replace("├─", "│") + "  │ "
                },
                if i == 0 { " emit " } else { "      " },
                i,
                Colour::Cyan.paint(format!("0x{}", hex::encode(&topic)))
            )
        }
        println!(
            "{}        data: {}",
            left.replace("├─", "│").replace("└─", "  ") + "  │  ",
            Colour::Cyan.paint(format!("0x{}", hex::encode(&log.data)))
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Ordering enum for calls and logs
///
/// i.e. if Call 0 occurs before Log 0, it will be pushed into the `CallTraceNode`'s ordering before
/// the log.
pub enum LogCallOrder {
    Log(usize),
    Call(usize),
}

/// Call trace of a tx
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct CallTrace {
    pub depth: usize,
    pub location: usize,
    pub idx: usize,
    /// Successful
    pub success: bool,
    /// Callee
    pub addr: H160,
    /// Creation
    pub created: bool,
    /// Ether value transfer
    pub value: U256,
    /// Call data, including function selector (if applicable)
    pub data: Vec<u8>,
    /// Gas cost
    pub cost: u64,
    /// Output
    pub output: Vec<u8>,
}

impl CallTrace {
    /// Updates a trace given another trace
    fn update(&mut self, new_trace: Self) {
        self.success = new_trace.success;
        self.addr = new_trace.addr;
        self.cost = new_trace.cost;
        self.output = new_trace.output;
        self.data = new_trace.data;
        self.addr = new_trace.addr;
    }

    /// Prints function call, returning the decoded or raw output
    pub fn print_func_call(
        &self,
        abi: Option<&Abi>,
        name: Option<&String>,
        color: Colour,
        left: &str,
    ) -> Output {
        if let (Some(abi), Some(name)) = (abi, name) {
            // Is data longer than 4, meaning we can attempt to decode it
            if self.data.len() >= 4 {
                for (func_name, overloaded_funcs) in abi.functions.iter() {
                    for func in overloaded_funcs.iter() {
                        if func.selector() == self.data[0..4] {
                            let mut strings = "".to_string();
                            if !self.data[4..].is_empty() {
                                let params = func
                                    .decode_input(&self.data[4..])
                                    .expect("Bad func data decode");
                                strings = params
                                    .into_iter()
                                    .map(format_token)
                                    .collect::<Vec<String>>()
                                    .join(", ");

                                #[cfg(feature = "sputnik")]
                                if self.addr == *CHEATCODE_ADDRESS && func.name == "expectRevert" {
                                    // try to decode better than just `bytes` for `expectRevert`
                                    if let Ok(decoded) = foundry_utils::decode_revert(&self.data) {
                                        strings = decoded;
                                    }
                                }
                            }

                            println!(
                                "{}[{}] {}::{}{}({})",
                                left,
                                self.cost,
                                color.paint(name),
                                color.paint(func_name),
                                if self.value > 0.into() {
                                    format!("{{value: {}}}", self.value)
                                } else {
                                    "".to_string()
                                },
                                strings,
                            );

                            if !self.output.is_empty() && self.success {
                                return Output::Token(
                                    func.decode_output(&self.output[..])
                                        .expect("Bad func output decode"),
                                )
                            } else if !self.output.is_empty() && !self.success {
                                if let Ok(decoded_error) =
                                    foundry_utils::decode_revert(&self.output[..])
                                {
                                    return Output::Token(vec![ethers::abi::Token::String(
                                        decoded_error,
                                    )])
                                } else {
                                    return Output::Raw(self.output.clone())
                                }
                            } else {
                                return Output::Raw(vec![])
                            }
                        }
                    }
                }
            } else {
                // fallback function
                println!(
                    "{}[{}] {}::fallback{}()",
                    left,
                    self.cost,
                    color.paint(name),
                    if self.value > 0.into() {
                        format!("{{value: {}}}", self.value)
                    } else {
                        "".to_string()
                    }
                );

                if !self.success {
                    if let Ok(decoded_error) = foundry_utils::decode_revert(&self.output[..]) {
                        return Output::Token(vec![ethers::abi::Token::String(decoded_error)])
                    }
                }
                return Output::Raw(self.output[..].to_vec())
            }
        }

        // We couldn't decode the function call, so print it as an abstract call
        println!(
            "{}[{}] {}::{}{}({})",
            left,
            self.cost,
            color.paint(format!("{}", self.addr)),
            if self.data.len() >= 4 {
                hex::encode(&self.data[0..4])
            } else {
                hex::encode(&self.data[..])
            },
            if self.value > 0.into() {
                format!("{{value: {}}}", self.value)
            } else {
                "".to_string()
            },
            if self.data.len() >= 4 {
                hex::encode(&self.data[4..])
            } else {
                hex::encode(&vec![][..])
            },
        );

        if !self.success {
            if let Ok(decoded_error) = foundry_utils::decode_revert(&self.output[..]) {
                return Output::Token(vec![ethers::abi::Token::String(decoded_error)])
            }
        }
        Output::Raw(self.output[..].to_vec())
    }
}

// Gets pretty print strings for tokens
fn format_token(param: ethers::abi::Token) -> String {
    use ethers::abi::Token;
    match param {
        Token::Address(addr) => format!("{:?}", addr),
        Token::FixedBytes(bytes) => format!("0x{}", hex::encode(&bytes)),
        Token::Bytes(bytes) => format!("0x{}", hex::encode(&bytes)),
        Token::Int(mut num) => {
            if num.bit(255) {
                num = num - 1;
                format!("-{}", num.overflowing_neg().0)
            } else {
                num.to_string()
            }
        }
        Token::Uint(num) => num.to_string(),
        Token::Bool(b) => format!("{}", b),
        Token::String(s) => s,
        Token::FixedArray(tokens) => {
            let string = tokens.into_iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{}]", string)
        }
        Token::Array(tokens) => {
            let string = tokens.into_iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("[{}]", string)
        }
        Token::Tuple(tokens) => {
            let string = tokens.into_iter().map(format_token).collect::<Vec<String>>().join(", ");
            format!("({})", string)
        }
    }
}

// very simple fuzzy matching to account for immutables. Will fail for small contracts that are
// basically all immutable vars
fn diff_score(bytecode1: &[u8], bytecode2: &[u8]) -> f64 {
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
}
