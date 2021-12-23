use ethers::{
    abi::{Abi, FunctionExt, RawLog},
    types::H160,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use ansi_term::Colour;

#[cfg(feature = "sputnik")]
use crate::sputnik::cheatcodes::{cheatcode_handler::CHEATCODE_ADDRESS, HEVM_ABI};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallTraceArena {
    pub arena: Vec<CallTraceNode>,
    pub entry: usize,
}

impl Default for CallTraceArena {
    fn default() -> Self {
        CallTraceArena {
            arena: vec![CallTraceNode {
                parent: None,
                children: vec![],
                idx: 0,
                trace: CallTrace::default(),
                logs: vec![],
                ordering: vec![],
            }],
            entry: 0,
        }
    }
}

pub enum Output {
    Token(Vec<ethers::abi::Token>),
    Raw(Vec<u8>),
}

impl CallTraceArena {
    pub fn push_trace(&mut self, entry: usize, mut new_trace: CallTrace) -> CallTrace {
        if new_trace.depth == 0 {
            // overwrite
            self.update(new_trace.clone());
            new_trace
        } else if self.arena[entry].trace.depth == new_trace.depth - 1 {
            new_trace.idx = self.arena.len();
            new_trace.location = self.arena[entry].children.len();
            self.arena[entry].ordering.push(LogCallOrder::Call(new_trace.location));
            let node = CallTraceNode {
                parent: Some(entry),
                children: vec![],
                idx: self.arena.len(),
                trace: new_trace.clone(),
                logs: vec![],
                ordering: vec![],
            };
            self.arena.push(node);
            self.arena[entry].children.push(new_trace.idx);
            new_trace
        } else {
            self.push_trace(
                *self.arena[entry].children.last().expect("Disconnected trace"),
                new_trace,
            )
        }
    }

    pub fn update(&mut self, trace: CallTrace) {
        let node = &mut self.arena[trace.idx];
        node.trace.update(trace);
    }

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

        let maybe_found;
        {
            if let Some((name, abi)) = identified_contracts.get(&trace.addr) {
                maybe_found = Some((name.clone(), abi.clone()));
            } else {
                maybe_found = None;
            }
        }
        if let Some((_name, _abi)) = maybe_found {
            if trace.created {
                self.update_children(idx, contracts, identified_contracts, evm);
                return
            }
            self.update_children(idx, contracts, identified_contracts, evm);
        } else {
            if trace.created {
                if let Some((name, (abi, _code))) = contracts
                    .iter()
                    .find(|(_key, (_abi, code))| diff_score(code, &trace.output) < 0.10)
                {
                    identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                    self.update_children(idx, contracts, identified_contracts, evm);
                    return
                } else {
                    self.update_children(idx, contracts, identified_contracts, evm);
                    return
                }
            }

            if let Some((name, (abi, _code))) = contracts
                .iter()
                .find(|(_key, (_abi, code))| diff_score(code, &evm.code(trace.addr)) < 0.10)
            {
                identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                // re-enter this function at this level if we found the contract
                self.update_identified(idx, contracts, identified_contracts, evm);
            } else {
                self.update_children(idx, contracts, identified_contracts, evm);
            }
        }
    }

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

    pub fn pretty_print<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        idx: usize,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
        left: String,
    ) {
        let trace = &self.arena[idx].trace;

        #[cfg(feature = "sputnik")]
        identified_contracts.insert(*CHEATCODE_ADDRESS, ("VM".to_string(), HEVM_ABI.clone()));

        #[cfg(feature = "sputnik")]
        let color = if trace.addr == *CHEATCODE_ADDRESS {
            Colour::Blue
        } else if trace.success {
            Colour::Green
        } else {
            Colour::Red
        };

        #[cfg(not(feature = "sputnik"))]
        let color = if trace.success { Colour::Green } else { Colour::Red };

        let mut abi = None;
        let mut name = None;
        {
            if let Some((name_, abi_)) = identified_contracts.get(&trace.addr) {
                abi = Some(abi_.clone());
                name = Some(name_.clone());
            }
        }
        // was this a contract creation?
        if trace.created {
            match (abi, name) {
                (Some(abi), Some(name)) => {
                    // if we have a match already, print it like normal
                    println!("{}{} {}@{}", left, Colour::Yellow.paint("→ new"), name, trace.addr);
                    self.print_children_and_logs(
                        idx,
                        Some(&abi),
                        contracts,
                        identified_contracts,
                        evm,
                        left.clone(),
                    );
                    println!(
                        "{}  └─ {} {} bytes of code",
                        left.replace("├─", "│").replace("└─", "  "),
                        color.paint("←"),
                        trace.output.len()
                    );
                }
                _ => {
                    // otherwise, try to identify it and print
                    if let Some((name, (abi, _code))) = contracts
                        .iter()
                        .find(|(_key, (_abi, code))| diff_score(code, &trace.output) < 0.10)
                    {
                        identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                        println!(
                            "{}{} {}@{}",
                            left,
                            Colour::Yellow.paint("→ new"),
                            name,
                            trace.addr
                        );
                        self.print_children_and_logs(
                            idx,
                            Some(abi),
                            contracts,
                            identified_contracts,
                            evm,
                            left.clone(),
                        );
                        println!(
                            "{}  └─ {} {} bytes of code",
                            left.replace("├─", "│").replace("└─", "  "),
                            color.paint("←"),
                            trace.output.len()
                        );
                    } else {
                        // we couldn't identify, print the children and logs without the abi
                        println!("{}→ new <Unknown>@{}", left, trace.addr);
                        self.print_children_and_logs(
                            idx,
                            None,
                            contracts,
                            identified_contracts,
                            evm,
                            left.clone(),
                        );
                        println!(
                            "{}  └─ {} {} bytes of code",
                            left.replace("├─", "│").replace("└─", "  "),
                            color.paint("←"),
                            trace.output.len()
                        );
                    }
                }
            }
        } else {
            match (abi, name) {
                (Some(abi), Some(name)) => {
                    let output =
                        Self::print_func_call(trace, Some(&abi), Some(&name), color, &left);
                    self.print_children_and_logs(
                        idx,
                        Some(&abi),
                        contracts,
                        identified_contracts,
                        evm,
                        left.clone(),
                    );
                    Self::print_output(color, output, left);
                }
                _ => {
                    if let Some((name, (abi, _code))) = contracts
                        .iter()
                        .find(|(_key, (_abi, code))| diff_score(code, &evm.code(trace.addr)) < 0.10)
                    {
                        identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                        // re-enter this function at this level if we found the contract
                        self.pretty_print(idx, contracts, identified_contracts, evm, left);
                    } else {
                        let output = Self::print_func_call(trace, None, None, color, &left);
                        self.print_children_and_logs(
                            idx,
                            None,
                            contracts,
                            identified_contracts,
                            evm,
                            left.clone(),
                        );
                        Self::print_output(color, output, left);
                    }
                }
            }
        }
    }

    pub fn print_children_and_logs<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        node_idx: usize,
        abi: Option<&Abi>,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
        left: String,
    ) {
        self.arena[node_idx].ordering.iter().for_each(|ordering| match ordering {
            LogCallOrder::Log(index) => {
                self.print_log(&self.arena[node_idx].logs[*index], abi, &left);
            }
            LogCallOrder::Call(index) => {
                self.pretty_print(
                    self.arena[node_idx].children[*index],
                    contracts,
                    identified_contracts,
                    evm,
                    left.replace("├─", "│").replace("└─", "  ") + "  ├─ ",
                );
            }
        });
    }

    /// Prints function call, optionally returning the decoded output
    pub fn print_func_call(
        trace: &CallTrace,
        abi: Option<&Abi>,
        name: Option<&String>,
        color: Colour,
        left: &str,
    ) -> Output {
        if let (Some(abi), Some(name)) = (abi, name) {
            if trace.data.len() >= 4 {
                for (func_name, overloaded_funcs) in abi.functions.iter() {
                    for func in overloaded_funcs.iter() {
                        if func.selector() == trace.data[0..4] {
                            let mut strings = "".to_string();
                            if !trace.data[4..].is_empty() {
                                let params = func
                                    .decode_input(&trace.data[4..])
                                    .expect("Bad func data decode");
                                strings = params
                                    .iter()
                                    .map(|param| format!("{}", param))
                                    .collect::<Vec<String>>()
                                    .join(", ");
                            }

                            println!(
                                "{}[{}] {}::{}({})",
                                left,
                                trace.cost,
                                color.paint(name),
                                color.paint(func_name),
                                strings
                            );

                            if !trace.output.is_empty() {
                                return Output::Token(
                                    func.decode_output(&trace.output[..])
                                        .expect("Bad func output decode"),
                                )
                            } else {
                                return Output::Raw(vec![])
                            }
                        }
                    }
                }
            } else {
                // fallback function
                println!("{}[{}] {}::fallback()", left, trace.cost, color.paint(name),);

                return Output::Raw(trace.output[..].to_vec())
            }
        }

        println!(
            "{}[{}] {}::{}({})",
            left,
            trace.cost,
            color.paint(format!("{}", trace.addr)),
            if trace.data.len() >= 4 {
                hex::encode(&trace.data[0..4])
            } else {
                hex::encode(&trace.data[..])
            },
            if trace.data.len() >= 4 {
                hex::encode(&trace.data[4..])
            } else {
                hex::encode(&vec![][..])
            }
        );

        Output::Raw(trace.output[..].to_vec())
    }

    pub fn print_log(&self, log: &RawLog, abi: Option<&Abi>, left: &str) {
        let right = "  ├─ ";
        if let Some(abi) = abi {
            for (event_name, overloaded_events) in abi.events.iter() {
                for event in overloaded_events.iter() {
                    if event.signature() == log.topics[0] {
                        let params = event.parse_log(log.clone()).expect("Bad event").params;
                        let strings = params
                            .iter()
                            .map(|param| format!("{}: {}", param.name, param.value))
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
                left.replace("├─", "│") + right,
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

    pub fn print_output(color: Colour, output: Output, left: String) {
        match output {
            Output::Token(token) => {
                let strings = token
                    .iter()
                    .map(|param| format!("{}", param))
                    .collect::<Vec<String>>()
                    .join(", ");
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallTraceNode {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub idx: usize,
    pub trace: CallTrace,
    /// Logs
    #[serde(skip)]
    pub logs: Vec<RawLog>,
    pub ordering: Vec<LogCallOrder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Call data, including function selector (if applicable)
    pub data: Vec<u8>,
    /// Gas cost
    pub cost: u64,
    /// Output
    pub output: Vec<u8>,
}

impl CallTrace {
    fn update(&mut self, new_trace: Self) {
        self.success = new_trace.success;
        self.addr = new_trace.addr;
        self.cost = new_trace.cost;
        self.output = new_trace.output;
        self.data = new_trace.data;
        self.addr = new_trace.addr;
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
