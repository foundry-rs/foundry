use crate::{
    executor::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    trace::{CallTraceArena, RawOrDecodedCall, TraceKind},
};
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, *};
use ethers::types::U256;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Display};

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

    pub fn analyze(&mut self, traces: &[(TraceKind, CallTraceArena)]) {
        let report_for_all = self.report_for.is_empty() || self.report_for.iter().any(|s| s == "*");
        traces.iter().for_each(|(_, trace)| {
            self.analyze_trace(trace, report_for_all);
        });
    }

    fn analyze_trace(&mut self, trace: &CallTraceArena, report_for_all: bool) {
        self.analyze_node(0, trace, report_for_all);
    }

    fn analyze_node(&mut self, node_index: usize, arena: &CallTraceArena, report_for_all: bool) {
        let node = &arena.arena[node_index];
        let trace = &node.trace;

        if trace.address == CHEATCODE_ADDRESS || trace.address == HARDHAT_CONSOLE_ADDRESS {
            return
        }

        if let Some(name) = &trace.contract {
            let report_for = self.report_for.iter().any(|s| s == name);
            if report_for || report_for_all {
                let mut contract_report =
                    self.contracts.entry(name.to_string()).or_insert_with(Default::default);

                match &trace.data {
                    RawOrDecodedCall::Raw(bytes) if trace.created() => {
                        contract_report.gas = trace.gas_cost.into();
                        contract_report.size = bytes.len().into();
                    }
                    // TODO: More robust test contract filtering
                    RawOrDecodedCall::Decoded(func, _)
                        if !func.starts_with("test") && func != "setUp" =>
                    {
                        let function_report = contract_report
                            .functions
                            .entry(func.clone())
                            .or_insert_with(Default::default);
                        function_report.calls.push(trace.gas_cost.into());
                    }
                    _ => (),
                }
            }
        }

        node.children.iter().for_each(|index| {
            self.analyze_node(*index, arena, report_for_all);
        });
    }

    #[must_use]
    pub fn finalize(mut self) -> Self {
        self.contracts.iter_mut().for_each(|(_, contract)| {
            contract.functions.iter_mut().for_each(|(_, func)| {
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
        self
    }
}

impl Display for GasReport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        for (name, contract) in self.contracts.iter() {
            if contract.functions.is_empty() {
                continue
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
            table.set_header(vec![Cell::new(format!("{name} contract"))
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
