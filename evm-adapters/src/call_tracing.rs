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
        evm: &'a E,
        left: String,
    ) {
        let trace = &self.arena[idx].trace;

        if trace.created {
            if let Some((name, (_abi, _code))) = contracts
                .iter()
                .find(|(_key, (_abi, code))| diff_score(code, &evm.code(trace.addr)) < 0.05)
            {
                println!("{}{} {}@{:?}", left, Colour::Yellow.paint("→ new"), name, trace.addr);
                return
            } else {
                println!("{}→ new <Unknown>@{:?}", left, trace.addr);
                return
            }
        }
        // fuzzy find contracts
        if let Some((name, (abi, _code))) = contracts
            .iter()
            .find(|(_key, (_abi, code))| diff_score(code, &evm.code(trace.addr)) < 0.05)
        {
            // color the printout by success
            let color = if trace.success { Colour::Green } else { Colour::Red };

            let mut decoded_output = None;
            // search thru abi functions to find matching
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

                        decoded_output = Some(
                            func.decode_output(&trace.output[..]).expect("Bad func output decode"),
                        );
                    }
                }
            }

            let children_idxs = &self.arena[idx].children;
            if children_idxs.len() == 0 {
                trace.prelogs.iter().for_each(|(log, loc)| {
                    if *loc == 0 {
                        let mut found = false;
                        let right = "  ├─ ";
                        'outer: for (event_name, overloaded_events) in abi.events.iter() {
                            for event in overloaded_events.iter() {
                                if event.signature() == log.topics[0] {
                                    found = true;
                                    let params =
                                        event.parse_log(log.clone()).expect("Bad event").params;
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
                    }
                });
            } else {
                children_idxs.iter().enumerate().for_each(|(i, child_idx)| {
                    let child_location = self.arena[*child_idx].trace.location;
                    trace.prelogs.iter().for_each(|(log, loc)| {
                        if loc == &child_location {
                            let mut found = false;
                            let right = "  ├─ ";
                            'outer: for (event_name, overloaded_events) in abi.events.iter() {
                                for event in overloaded_events.iter() {
                                    if event.signature() == log.topics[0] {
                                        found = true;
                                        let params =
                                            event.parse_log(log.clone()).expect("Bad event").params;
                                        let strings = params
                                            .iter()
                                            .map(|param| {
                                                format!("{}: {:?}", param.name, param.value)
                                            })
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
                        }
                    });
                    if i == children_idxs.len() - 1 &&
                        trace.logs.len() == 0 &&
                        decoded_output.is_none()
                    {
                        self.pretty_print(
                            *child_idx,
                            contracts,
                            evm,
                            left.to_string().replace("├─", "│").replace("└─", "  ") + "  └─ ",
                        );
                    } else {
                        self.pretty_print(
                            *child_idx,
                            contracts,
                            evm,
                            left.to_string().replace("├─", "│").replace("└─", "  ") + "  ├─ ",
                        );
                    }
                });
            }

            trace.logs.iter().enumerate().for_each(|(i, log)| {
                let mut found = false;
                let mut right = "  ├─ ";
                if i == trace.logs.len() - 1 && decoded_output.is_none() {
                    right = "  └─ ";
                }
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

            if let Some(decoded) = decoded_output {
                let strings = decoded
                    .iter()
                    .map(|param| format!("{:?}", param))
                    .collect::<Vec<String>>()
                    .join(", ");
                println!(
                    "{}  └─ {} {}",
                    left.to_string().replace("├─", "│").replace("└─", "  "),
                    Colour::Green.paint("←"),
                    if strings.len() == 0 { "()" } else { &*strings }
                );
            }
        } else {
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
                        evm,
                        left.to_string().replace("├─", "│").replace("└─", "  ") + " _└─ ",
                    );
                } else {
                    self.pretty_print(
                        *child_idx,
                        contracts,
                        evm,
                        left.to_string().replace("├─", "│").replace("└─", "  ") + " _├─ ",
                    );
                }
            });

            let mut right = " _├─ ";

            trace.logs.iter().enumerate().for_each(|(i, log)| {
                if i == trace.logs.len() - 1 {
                    right = " _└─ ";
                }
                println!(
                    "{}emit {}",
                    left.to_string().replace("├─", "│") + right,
                    Colour::Cyan.paint(format!("{:?}", log))
                )
            });
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
    /// inner calls
    pub inner: Vec<CallTrace>,
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
        // we dont update inner because the temporary new_trace doesnt track inner calls
    }

    // pub fn inner_number_of_logs(&self) -> usize {
    //     // only count child logs
    //     let mut total = 0;
    //     if self.inner.len() > 0 {
    //         self.inner.iter().for_each(|inner| {
    //             total += inner.inner_number_of_logs();
    //         });
    //     }
    //     total += self.logs.len() + self.prelogs.len();
    //     total
    // }

    // pub fn logs_up_to_depth_loc(&self, depth: usize, loc: usize) -> usize {
    // 	if depth == 0 {
    // 		return 0;
    // 	}
    // 	let target = self.get_trace(depth, loc).expect("huh?");
    // 	let parent = self.get_trace(target.depth - 1, target.parent_location).expect("huh?");
    // 	let siblings = &parent.inner[..loc];
    // 	let mut total = target.logs.len() + target.prelogs.len();
    // 	for sibling in siblings.iter() {
    // 		total += sibling.logs.len() + sibling.prelogs.len();
    // 	}
    // 	total += self.logs_up_to_depth_loc(target.depth - 1, target.parent_location);
    // 	total
    // }

    // pub fn next_log_index(&self, depth: usize, loc: usize) -> usize {
    // 	let total = self.logs_up_to_depth_loc(depth, loc);
    // 	let target = self.get_trace(depth, loc).expect("huh?");
    // 	total + target.inner_number_of_logs()
    // }

    // pub fn inner_number_of_inners(&self) -> usize {
    //     // only count child logs
    //     let mut total = 0;
    //     if self.inner.len() > 0 {
    //         self.inner.iter().for_each(|inner| {
    //             total += inner.inner_number_of_inners();
    //         });
    //     }
    //     total += self.inner.len();
    //     total
    // }

    // pub fn get_trace(&self, depth: usize, location: usize) -> Option<&CallTrace> {
    //     if self.depth == depth && self.location == location {
    //         return Some(&self)
    //     } else {
    //         if self.depth != depth {
    //             for inner in self.inner.iter() {
    //                 if let Some(trace) = inner.get_trace(depth, location) {
    //                     return Some(trace)
    //                 }
    //             }
    //         }
    //     }
    //     return None
    // }

    // pub fn get_trace_mut(&mut self, depth: usize, location: usize) -> Option<&mut CallTrace> {
    //     if self.depth == depth && self.location == location {
    //         return Some(self)
    //     } else {
    //         if self.depth != depth {
    //             for inner in self.inner.iter_mut() {
    //                 if let Some(trace) = inner.get_trace_mut(depth, location) {
    //                     return Some(trace)
    //                 }
    //             }
    //         }
    //     }
    //     return None
    // }
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
