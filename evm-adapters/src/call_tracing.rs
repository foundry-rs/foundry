use ethers::{
    abi::{Abi, Event, Function, RawLog, Token},
    types::{H160, H256, U256},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use ansi_term::Colour;

use foundry_utils::format_token;

#[cfg(feature = "sputnik")]
use crate::sputnik::cheatcodes::{
    cheatcode_handler::{CHEATCODE_ADDRESS, CONSOLE_ADDRESS},
    CONSOLE_ABI, HEVM_ABI,
};

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
    pub identified_contracts: &'a mut BTreeMap<H160, (String, Abi)>,
    pub labeled_addrs: &'a BTreeMap<H160, String>,
    pub funcs: &'a BTreeMap<[u8; 4], Function>,
    pub events: &'a BTreeMap<H256, Event>,
    pub errors: &'a Abi,
}

impl<'a> ExecutionInfo<'a> {
    pub fn new(
        contracts: &'a BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &'a mut BTreeMap<H160, (String, Abi)>,
        labeled_addrs: &'a BTreeMap<H160, String>,
        funcs: &'a BTreeMap<[u8; 4], Function>,
        events: &'a BTreeMap<H256, Event>,
        errors: &'a Abi,
    ) -> Self {
        Self { contracts, identified_contracts, labeled_addrs, funcs, events, errors }
    }
}

impl Output {
    pub fn construct_string<'a, 'b>(
        self,
        exec_info: &'b ExecutionInfo<'a>,
        color: Colour,
        left: &str,
        full_str: &mut String,
    ) {
        match self {
            Output::Token(token) => {
                let strings = token
                    .iter()
                    .map(|token| format_labeled_token(token, exec_info))
                    .collect::<Vec<_>>()
                    .join(", ");
                full_str.push_str(&*format!(
                    "\n{}  └─ {} {}",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    if strings.is_empty() { "()" } else { &*strings }
                ));
            }
            Output::Raw(bytes) => {
                full_str.push_str(&*format!(
                    "\n{}  └─ {} {}",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    if bytes.is_empty() {
                        "()".to_string()
                    } else {
                        "0x".to_string() + &hex::encode(&bytes)
                    }
                ));
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

    /// Construct a CallTraceArena trace string
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
    pub fn construct_trace_string<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        idx: usize,
        exec_info: &mut ExecutionInfo<'a>,
        evm: &'a E,
        left: &str,
        full_str: &mut String,
    ) {
        let trace = &self.arena[idx].trace;

        #[cfg(feature = "sputnik")]
        {
            exec_info
                .identified_contracts
                .insert(*CHEATCODE_ADDRESS, ("VM".to_string(), HEVM_ABI.clone()));
            exec_info
                .identified_contracts
                .insert(*CONSOLE_ADDRESS, ("console".to_string(), CONSOLE_ABI.clone()));
        }

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
        let res = if let Some((name, abi)) = exec_info.identified_contracts.get(&trace.addr) {
            Some((name.clone(), abi.clone()))
        } else {
            None
        };
        if res.is_none() {
            // get the code to compare
            let code = if trace.created { trace.output.clone() } else { evm.code(trace.addr) };
            if let Some((name, (abi, _code))) = exec_info
                .contracts
                .iter()
                .find(|(_key, (_abi, known_code))| diff_score(known_code, &code) < 0.10)
            {
                // found matching contract, insert and print
                exec_info.identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                if trace.created {
                    full_str.push_str(&*format!(
                        "\n{}{} {}@{}",
                        left,
                        Colour::Yellow.paint("→ new"),
                        name,
                        trace.addr
                    ));
                    self.construct_children_and_logs(idx, exec_info, evm, left, full_str);
                    full_str.push_str(&*format!(
                        "\n{}  └─ {} {} bytes of code",
                        left.replace("├─", "│").replace("└─", "  "),
                        color.paint("←"),
                        trace.output.len()
                    ));
                } else {
                    // re-enter this function at the current node
                    self.construct_trace_string(idx, exec_info, evm, left, full_str);
                }
            } else if trace.created {
                // we couldn't identify, print the children and logs without the abi
                full_str.push_str(&*format!(
                    "\n{}{} <Unknown>@{}",
                    left,
                    Colour::Yellow.paint("→ new"),
                    trace.addr
                ));
                self.construct_children_and_logs(idx, exec_info, evm, left, full_str);
                full_str.push_str(&*format!(
                    "\n{}  └─ {} {} bytes of code",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    trace.output.len()
                ));
            } else {
                let output = trace.construct_func_call(exec_info, None, color, left, full_str);
                self.construct_children_and_logs(idx, exec_info, evm, left, full_str);
                output.construct_string(exec_info, color, left, full_str);
            }
        } else if let Some((name, _abi)) = res {
            if trace.created {
                full_str.push_str(&*format!(
                    "\n{}{} {}@{}",
                    left,
                    Colour::Yellow.paint("→ new"),
                    name,
                    trace.addr
                ));
                self.construct_children_and_logs(idx, exec_info, evm, left, full_str);
                full_str.push_str(&*format!(
                    "\n{}  └─ {} {} bytes of code",
                    left.replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    trace.output.len()
                ));
            } else {
                let output =
                    trace.construct_func_call(exec_info, Some(&name), color, left, full_str);
                self.construct_children_and_logs(idx, exec_info, evm, left, full_str);
                output.construct_string(exec_info, color, left, full_str);
            }
        }
    }

    /// Prints child calls and logs in order
    pub fn construct_children_and_logs<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        node_idx: usize,
        exec_info: &mut ExecutionInfo<'a>,
        evm: &'a E,
        left: &str,
        full_str: &mut String,
    ) {
        // Ordering stores a vec of `LogCallOrder` which is populated based on if
        // a log or a call was called first. This makes it such that we always print
        // logs and calls in the correct order
        self.arena[node_idx].ordering.iter().for_each(|ordering| match ordering {
            LogCallOrder::Log(index) => {
                self.arena[node_idx].construct_log(
                    exec_info,
                    *index,
                    exec_info.events,
                    left,
                    full_str,
                );
            }
            LogCallOrder::Call(index) => {
                self.construct_trace_string(
                    self.arena[node_idx].children[*index],
                    exec_info,
                    evm,
                    &(left.replace("├─", "│").replace("└─", "  ") + "  ├─ "),
                    full_str,
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
    pub fn construct_log<'a, 'b>(
        &self,
        exec_info: &'b ExecutionInfo<'a>,
        index: usize,
        events: &BTreeMap<H256, Event>,
        left: &str,
        full_str: &mut String,
    ) {
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
                full_str.push_str(&*format!(
                    "\n{}emit {}({})",
                    left.replace("├─", "│") + right,
                    Colour::Cyan.paint(event.name.clone()),
                    strings
                ));
                return
            }
        }

        // we didnt decode the log, print it as an unknown log
        for (i, topic) in log.topics.iter().enumerate() {
            let right = if i == log.topics.len() - 1 && log.data.is_empty() {
                "  └─ "
            } else {
                "  ├─"
            };
            full_str.push_str(&*format!(
                "\n{}{}topic {}: {}",
                if i == 0 {
                    left.replace("├─", "│") + right
                } else {
                    left.replace("├─", "│") + "  │ "
                },
                if i == 0 { " emit " } else { "      " },
                i,
                Colour::Cyan.paint(format!("0x{}", hex::encode(&topic)))
            ))
        }
        full_str.push_str(&*format!(
            "\n{}        data: {}",
            left.replace("├─", "│").replace("└─", "  ") + "  │  ",
            Colour::Cyan.paint(format!("0x{}", hex::encode(&log.data)))
        ))
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
    /// Label for an address
    pub label: Option<String>,
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
    pub fn construct_func_call<'a>(
        &self,
        exec_info: &mut ExecutionInfo<'a>,
        name: Option<&String>,
        color: Colour,
        left: &str,
        full_str: &mut String,
    ) -> Output {
        // Is data longer than 4, meaning we can attempt to decode it
        if self.data.len() >= 4 {
            if let Some(func) = exec_info.funcs.get(&self.data[0..4]) {
                let mut strings = "".to_string();
                if !self.data[4..].is_empty() {
                    let params = func.decode_input(&self.data[4..]).expect("Bad func data decode");
                    strings = params
                        .iter()
                        .map(|token| format_labeled_token(token, exec_info))
                        .collect::<Vec<_>>()
                        .join(", ");

                    #[cfg(feature = "sputnik")]
                    if self.addr == *CHEATCODE_ADDRESS && func.name == "expectRevert" {
                        // try to decode better than just `bytes` for `expectRevert`
                        if let Ok(decoded) =
                            foundry_utils::decode_revert(&self.data, Some(exec_info.errors))
                        {
                            strings = decoded;
                        }
                    }
                }

                full_str.push_str(&*format!(
                    "\n{}[{}] {}::{}{}({})",
                    left,
                    self.cost,
                    color.paint(
                        // clippy bug makes us do this
                        #[allow(clippy::or_fun_call)]
                        self.label.as_ref().unwrap_or(name.unwrap_or(&self.addr.to_string()))
                    ),
                    color.paint(func.name.clone()),
                    if self.value > 0.into() {
                        format!("{{value: {}}}", self.value)
                    } else {
                        "".to_string()
                    },
                    strings,
                ));

                if !self.output.is_empty() && self.success {
                    return Output::Token(
                        func.decode_output(&self.output[..]).expect("Bad func output decode"),
                    )
                } else if !self.output.is_empty() && !self.success {
                    if let Ok(decoded_error) =
                        foundry_utils::decode_revert(&self.output[..], Some(exec_info.errors))
                    {
                        return Output::Token(vec![ethers::abi::Token::String(decoded_error)])
                    } else {
                        return Output::Raw(self.output.clone())
                    }
                } else {
                    return Output::Raw(vec![])
                }
            }
        } else {
            // fallback function
            full_str.push_str(&*format!(
                "\n{}[{}] {}::fallback{}()",
                left,
                self.cost,
                color.paint(
                    // clippy bug makes us do this
                    #[allow(clippy::or_fun_call)]
                    self.label.as_ref().unwrap_or(name.unwrap_or(&self.addr.to_string()))
                ),
                if self.value > 0.into() {
                    format!("{{value: {}}}", self.value)
                } else {
                    "".to_string()
                }
            ));

            if !self.success {
                if let Ok(decoded_error) =
                    foundry_utils::decode_revert(&self.output[..], Some(exec_info.errors))
                {
                    return Output::Token(vec![ethers::abi::Token::String(decoded_error)])
                }
            }
            return Output::Raw(self.output[..].to_vec())
        }

        // We couldn't decode the function call, so print it as an abstract call
        full_str.push_str(&*format!(
            "\n{}[{}] {}::{}{}({})",
            left,
            self.cost,
            color.paint(self.label.as_ref().unwrap_or(&self.addr.to_string()).to_string()),
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
        ));

        if !self.success {
            if let Ok(decoded_error) =
                foundry_utils::decode_revert(&self.output[..], Some(exec_info.errors))
            {
                return Output::Token(vec![ethers::abi::Token::String(decoded_error)])
            }
        }
        Output::Raw(self.output[..].to_vec())
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
