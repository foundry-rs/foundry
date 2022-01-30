use crate::CallTraceArena;
use ethers::{
    abi::Abi,
    types::{H160, U256},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Display};

#[cfg(feature = "sputnik")]
use crate::sputnik::cheatcodes::cheatcode_handler::{CHEATCODE_ADDRESS, CONSOLE_ADDRESS};

use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, *};

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct GasReport {
    pub report_for: Vec<String>,
    pub contracts: BTreeMap<String, ContractInfo>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ContractInfo {
    pub gas: U256,
    pub size: U256,
    pub functions: BTreeMap<String, GasInfo>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct GasInfo {
    pub calls: Vec<U256>,
    pub min: U256,
    pub mean: U256,
    pub median: U256,
    pub max: U256,
}

impl GasReport {
    pub fn new(report_for: Vec<String>) -> Self {
        Self { report_for, ..Default::default() }
    }

    pub fn analyze(
        &mut self,
        traces: &[CallTraceArena],
        identified_contracts: &BTreeMap<H160, (String, Abi)>,
    ) {
        let report_for_all = self.report_for.is_empty() || self.report_for.iter().any(|s| s == "*");
        traces.iter().for_each(|trace| {
            self.analyze_trace(trace, identified_contracts, report_for_all);
        });
    }

    fn analyze_trace(
        &mut self,
        trace: &CallTraceArena,
        identified_contracts: &BTreeMap<H160, (String, Abi)>,
        report_for_all: bool,
    ) {
        self.analyze_node(trace.entry, trace, identified_contracts, report_for_all);
    }

    fn analyze_node(
        &mut self,
        node_index: usize,
        arena: &CallTraceArena,
        identified_contracts: &BTreeMap<H160, (String, Abi)>,
        report_for_all: bool,
    ) {
        let node = &arena.arena[node_index];
        let trace = &node.trace;

        #[cfg(feature = "sputnik")]
        if trace.addr == *CHEATCODE_ADDRESS || trace.addr == *CONSOLE_ADDRESS {
            return
        }

        if let Some((name, abi)) = identified_contracts.get(&trace.addr) {
            let report_for = self.report_for.iter().any(|s| s == name);
            if !report_for && abi.functions().any(|func| func.name == "IS_TEST") {
                // do nothing
            } else if report_for || report_for_all {
                // report for this contract
                let mut contract =
                    self.contracts.entry(name.to_string()).or_insert_with(Default::default);

                if trace.created {
                    contract.gas = trace.cost.into();
                    contract.size = trace.data.len().into();
                } else if trace.data.len() >= 4 {
                    let func =
                        abi.functions().find(|func| func.short_signature() == trace.data[0..4]);

                    if let Some(func) = func {
                        let function = contract
                            .functions
                            .entry(func.name.clone())
                            .or_insert_with(Default::default);
                        function.calls.push(trace.cost.into());
                    }
                }
            }
        }
        node.children.iter().for_each(|index| {
            self.analyze_node(*index, arena, identified_contracts, report_for_all);
        });
    }

    pub fn finalize(&mut self) {
        self.contracts.iter_mut().for_each(|(_name, contract)| {
            contract.functions.iter_mut().for_each(|(_name, func)| {
                func.calls.sort();
                func.min = func.calls.first().cloned().unwrap_or_default();
                func.max = func.calls.last().cloned().unwrap_or_default();
                func.mean =
                    func.calls.iter().fold(U256::zero(), |acc, x| acc + x) / func.calls.len();

                let len = func.calls.len();
                func.median = if len > 0 {
                    if len % 2 == 0 {
                        (func.calls[len / 2 - 1] + func.calls[len / 2]) / 2
                    } else {
                        func.calls[len / 2]
                    }
                } else {
                    0.into()
                };
            });
        });
    }
}

impl Display for GasReport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        for (name, contract) in self.contracts.iter() {
            let mut table = Table::new();
            table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
            table.set_header(vec![Cell::new(format!("{} contract", name))
                .add_attribute(Attribute::Bold)
                .fg(Color::Green)]);
            table.add_row(vec![
                Cell::new("Deployment Cost").add_attribute(Attribute::Bold).fg(Color::Cyan),
                Cell::new("Deployment Size").add_attribute(Attribute::Bold).fg(Color::Cyan),
            ]);
            table.add_row(vec![contract.gas.to_string(), contract.size.to_string()]);

            table.add_row(vec![
                Cell::new("Function Name").add_attribute(Attribute::Bold).fg(Color::Magenta),
                Cell::new("min").add_attribute(Attribute::Bold).fg(Color::Green),
                Cell::new("avg").add_attribute(Attribute::Bold).fg(Color::Yellow),
                Cell::new("median").add_attribute(Attribute::Bold).fg(Color::Yellow),
                Cell::new("max").add_attribute(Attribute::Bold).fg(Color::Red),
                Cell::new("# calls").add_attribute(Attribute::Bold),
            ]);
            contract.functions.iter().for_each(|(fname, function)| {
                table.add_row(vec![
                    Cell::new(fname.to_string()).add_attribute(Attribute::Bold),
                    Cell::new(function.min.to_string()).fg(Color::Green),
                    Cell::new(function.mean.to_string()).fg(Color::Yellow),
                    Cell::new(function.median.to_string()).fg(Color::Yellow),
                    Cell::new(function.max.to_string()).fg(Color::Red),
                    Cell::new(function.calls.len().to_string()),
                ]);
            });
            writeln!(f, "{}", table)?
        }
        Ok(())
    }
}
