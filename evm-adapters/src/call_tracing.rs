use ethers::{
    abi::{Abi, FunctionExt, RawLog},
    types::H160,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use ansi_term::Colour;

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
            let node = CallTraceNode {
                parent: Some(entry),
                children: vec![],
                idx: self.arena.len(),
                trace: new_trace.clone(),
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

    pub fn inner_number_of_logs(&self, node_idx: usize) -> usize {
        self.arena[node_idx].children.iter().fold(0, |accum, idx| {
            accum +
                self.arena[*idx].trace.prelogs.len() +
                self.arena[*idx].trace.logs.len() +
                self.inner_number_of_logs(*idx)
        }) //+ self.arena[node_idx].trace.prelogs.len() + self.arena[node_idx].trace.logs.len()
    }

    pub fn inner_number_of_inners(&self, node_idx: usize) -> usize {
        self.arena[node_idx].children.iter().fold(0, |accum, idx| {
            accum + self.arena[*idx].children.len() + self.inner_number_of_inners(*idx)
        })
    }

    pub fn next_log_index(&self) -> usize {
        self.inner_number_of_logs(self.entry)
    }

    pub fn print_logs_in_order(&self, node_idx: usize) {
        self.arena[node_idx]
            .trace
            .prelogs
            .iter()
            .for_each(|(raw, _loc)| println!("prelog {}", raw.topics[0]));
        self.arena[node_idx].children.iter().for_each(|idx| {
            self.print_logs_in_order(*idx);
        });
        self.arena[node_idx].trace.logs.iter().for_each(|raw| println!("log {}", raw.topics[0]));
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

        // color the printout by success
        let color = if trace.success { Colour::Green } else { Colour::Red };

        let maybe_found;
        {
            if let Some((name, abi)) = identified_contracts.get(&trace.addr) {
                maybe_found = Some((name.clone(), abi.clone()));
            } else {
                maybe_found = None;
            }
        }
        if let Some((name, abi)) = maybe_found {
            if trace.created {
                println!("{}{} {}@{:?}", left, Colour::Yellow.paint("→ new"), name, trace.addr);
                self.print_children_and_prelogs(
                    idx,
                    trace,
                    &abi,
                    contracts,
                    identified_contracts,
                    evm,
                    left.clone(),
                );
                Self::print_logs(trace, &abi, &left);
                println!(
                    "{}  └─ {} {} bytes of code",
                    left.to_string().replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    trace.output.len() / 2
                );
                return
            }
            let output = Self::print_func_call(trace, &abi, &name, color, &left);
            self.print_children_and_prelogs(
                idx,
                trace,
                &abi,
                contracts,
                identified_contracts,
                evm,
                left.clone(),
            );
            Self::print_logs(trace, &abi, &left);
            Self::print_output(color, output, left);
        } else {
            if trace.created {
                if let Some((name, (abi, _code))) = contracts
                    .iter()
                    .find(|(_key, (_abi, code))| diff_score(code, &trace.output) < 0.10)
                {
                    identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                    println!("{}{} {}@{:?}", left, Colour::Yellow.paint("→ new"), name, trace.addr);
                    self.print_children_and_prelogs(
                        idx,
                        trace,
                        &abi,
                        contracts,
                        identified_contracts,
                        evm,
                        left.clone(),
                    );
                    Self::print_logs(trace, &abi, &left);
                    println!(
                        "{}  └─ {} {} bytes of code",
                        left.to_string().replace("├─", "│").replace("└─", "  "),
                        color.paint("←"),
                        trace.output.len() / 2
                    );
                    return
                } else {
                    println!("{}→ new <Unknown>@{:?}", left, trace.addr);
                    self.print_unknown(
                        color,
                        idx,
                        trace,
                        contracts,
                        identified_contracts,
                        evm,
                        left.clone(),
                    );
                    return
                }
            }

            if let Some((name, (abi, _code))) = contracts
                .iter()
                .find(|(_key, (_abi, code))| diff_score(code, &evm.code(trace.addr)) < 0.10)
            {
                identified_contracts.insert(trace.addr, (name.to_string(), abi.clone()));
                // re-enter this function at this level if we found the contract
                self.pretty_print(idx, contracts, identified_contracts, evm, left);
            } else {
                self.print_unknown(
                    color,
                    idx,
                    trace,
                    contracts,
                    identified_contracts,
                    evm,
                    left.clone(),
                );
            }
        }
    }

    pub fn print_unknown<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        color: Colour,
        idx: usize,
        trace: &CallTrace,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
        left: String,
    ) {
        if trace.data.len() >= 4 {
            println!(
                "{}{:x}::{}({})",
                left,
                trace.addr,
                hex::encode(&trace.data[0..4]),
                hex::encode(&trace.data[4..])
            );
        } else {
            println!("{}{:x}::({})", left, trace.addr, hex::encode(&trace.data));
        }

        let children_idxs = &self.arena[idx].children;
        children_idxs.iter().enumerate().for_each(|(i, child_idx)| {
            // let inners = inner.inner_number_of_inners();
            if i == children_idxs.len() - 1 && trace.logs.len() == 0 {
                self.pretty_print(
                    *child_idx,
                    contracts,
                    identified_contracts,
                    evm,
                    left.to_string().replace("├─", "│").replace("└─", "  ") + "  └─ ",
                );
            } else {
                self.pretty_print(
                    *child_idx,
                    contracts,
                    identified_contracts,
                    evm,
                    left.to_string().replace("├─", "│").replace("└─", "  ") + "  ├─ ",
                );
            }
        });

        let mut right = "  ├─ ";

        trace.logs.iter().enumerate().for_each(|(i, log)| {
            if i == trace.logs.len() - 1 {
                right = "  └─ ";
            }
            println!(
                "{}emit {}",
                left.to_string().replace("├─", "│") + right,
                Colour::Cyan.paint(format!("{:?}", log))
            )
        });

        if !trace.created {
            println!(
                "{}  └─ {} {}",
                left.to_string().replace("├─", "│").replace("└─", "  "),
                color.paint("←"),
                if trace.output.len() == 0 {
                    "()".to_string()
                } else {
                    "0x".to_string() + &hex::encode(&trace.output)
                }
            );
        } else {
            println!(
                "{}  └─ {} {} bytes of code",
                left.to_string().replace("├─", "│").replace("└─", "  "),
                color.paint("←"),
                trace.output.len() / 2
            );
        }
    }

    /// Prints function call, optionally returning the decoded output
    pub fn print_func_call(
        trace: &CallTrace,
        abi: &Abi,
        name: &String,
        color: Colour,
        left: &String,
    ) -> Output {
        if trace.data.len() >= 4 {
            for (func_name, overloaded_funcs) in abi.functions.iter() {
                for func in overloaded_funcs.iter() {
                    if func.selector() == trace.data[0..4] {
                        let params =
                            func.decode_input(&trace.data[4..]).expect("Bad func data decode");
                        let strings = params
                            .iter()
                            .map(|param| format!("{:?}", param))
                            .collect::<Vec<String>>()
                            .join(", ");

                        println!(
                            "{}[{}] {}::{}({})",
                            left,
                            trace.cost,
                            color.paint(name),
                            color.paint(func_name),
                            strings
                        );

                        return Output::Token(
                            func.decode_output(&trace.output[..]).expect("Bad func output decode"),
                        )
                    }
                }
            }
        } else {
            // fallback function?
            println!("{}[{}] {}::fallback()", left, trace.cost, color.paint(name),);

            return Output::Raw(trace.output[..].to_vec())
        }

        println!(
            "{}[{}] {}::{}({})",
            left,
            trace.cost,
            color.paint(name),
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

    pub fn print_prelogs(
        prelogs: &Vec<(RawLog, usize)>,
        location: usize,
        abi: &Abi,
        left: &String,
    ) {
        prelogs.iter().for_each(|(log, loc)| {
            if *loc == location {
                let mut found = false;
                let right = "  ├─ ";
                'outer: for (event_name, overloaded_events) in abi.events.iter() {
                    for event in overloaded_events.iter() {
                        if event.signature() == log.topics[0] {
                            found = true;
                            let params = event.parse_log(log.clone()).expect("Bad event").params;
                            let strings = params
                                .iter()
                                .map(|param| format!("{}: {:?}", param.name, param.value))
                                .collect::<Vec<String>>()
                                .join(", ");
                            println!(
                                "{}emit {}({})",
                                left.to_string().replace("├─", "│") + right,
                                Colour::Blue.paint(event_name),
                                strings
                            );
                            break 'outer
                        }
                    }
                }
                if !found {
                    println!(
                        "{}emit {}",
                        left.to_string().replace("├─", "│") + right,
                        Colour::Blue.paint(format!("{:?}", log))
                    )
                }
            }
        });
    }

    pub fn print_children_and_prelogs<'a, S: Clone, E: crate::Evm<S>>(
        &self,
        idx: usize,
        trace: &CallTrace,
        abi: &Abi,
        contracts: &BTreeMap<String, (Abi, Vec<u8>)>,
        identified_contracts: &mut BTreeMap<H160, (String, Abi)>,
        evm: &'a E,
        left: String,
    ) {
        let children_idxs = &self.arena[idx].children;
        if children_idxs.len() == 0 {
            Self::print_prelogs(&trace.prelogs, 0, abi, &left);
        } else {
            children_idxs.iter().for_each(|child_idx| {
                let child_location = self.arena[*child_idx].trace.location;
                Self::print_prelogs(&trace.prelogs, child_location, abi, &left);
                self.pretty_print(
                    *child_idx,
                    contracts,
                    identified_contracts,
                    evm,
                    left.to_string().replace("├─", "│").replace("└─", "  ") + "  ├─ ",
                );
            });
        }
    }

    pub fn print_logs(trace: &CallTrace, abi: &Abi, left: &String) {
        trace.logs.iter().for_each(|log| {
            let mut found = false;
            let right = "  ├─ ";
            'outer: for (event_name, overloaded_events) in abi.events.iter() {
                for event in overloaded_events.iter() {
                    if event.signature() == log.topics[0] {
                        found = true;
                        let params = event.parse_log(log.clone()).expect("Bad event").params;
                        let strings = params
                            .iter()
                            .map(|param| format!("{}: {:?}", param.name, param.value))
                            .collect::<Vec<String>>()
                            .join(", ");
                        println!(
                            "{}emit {}({})",
                            left.to_string().replace("├─", "│") + right,
                            Colour::Cyan.paint(event_name),
                            strings
                        );
                        break 'outer
                    }
                }
            }
            if !found {
                println!(
                    "{}emit {}",
                    left.to_string().replace("├─", "│") + right,
                    Colour::Cyan.paint(format!("{:?}", log))
                )
            }
        });
    }

    pub fn print_output(color: Colour, output: Output, left: String) {
        match output {
            Output::Token(token) => {
                let strings = token
                    .iter()
                    .map(|param| format!("{:?}", param))
                    .collect::<Vec<String>>()
                    .join(", ");
                println!(
                    "{}  └─ {} {}",
                    left.to_string().replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    if strings.len() == 0 { "()" } else { &*strings }
                );
            }
            Output::Raw(bytes) => {
                println!(
                    "{}  └─ {} {}",
                    left.to_string().replace("├─", "│").replace("└─", "  "),
                    color.paint("←"),
                    if bytes.len() == 0 {
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
    /// Logs emitted before inner
    #[serde(skip)]
    pub prelogs: Vec<(RawLog, usize)>,
    /// Logs
    #[serde(skip)]
    pub logs: Vec<RawLog>,
}

impl CallTrace {
    fn update(&mut self, new_trace: Self) {
        self.success = new_trace.success;
        self.addr = new_trace.addr;
        self.cost = new_trace.cost;
        self.output = new_trace.output;
        self.logs = new_trace.logs;
        self.data = new_trace.data;
        self.addr = new_trace.addr;
    }
}

// very simple fuzzy matching to account for immutables. Will fail for small contracts that are
// basically all immutable vars
fn diff_score(bytecode1: &Vec<u8>, bytecode2: &Vec<u8>) -> f64 {
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

    diff_chars as f64 / cutoff_len as f64
}
