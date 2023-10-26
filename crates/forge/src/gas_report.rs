use crate::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    traces::{CallTraceArena, RawOrDecodedCall, TraceKind},
};
use alloy_primitives::U256;
use comfy_table::{presets::ASCII_MARKDOWN, *};
use foundry_common::{calc, TestFunctionExt};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Display};

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct GasReport {
    pub report_for: Vec<String>,
    pub ignore: Vec<String>,
    pub contracts: BTreeMap<String, ContractInfo>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ContractInfo {
    pub gas: U256,
    pub size: U256,
    pub functions: BTreeMap<String, BTreeMap<String, GasInfo>>,
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
    pub fn new(report_for: Vec<String>, ignore: Vec<String>) -> Self {
        Self { report_for, ignore, ..Default::default() }
    }

    pub fn analyze(&mut self, traces: &[(TraceKind, CallTraceArena)]) {
        traces.iter().for_each(|(_, trace)| {
            self.analyze_node(0, trace);
        });
    }

    fn analyze_node(&mut self, node_index: usize, arena: &CallTraceArena) {
        let node = &arena.arena[node_index];
        let trace = &node.trace;

        if trace.address == CHEATCODE_ADDRESS || trace.address == HARDHAT_CONSOLE_ADDRESS {
            return
        }

        if let Some(name) = &trace.contract {
            let contract_name = name.rsplit(':').next().unwrap_or(name.as_str()).to_string();
            // If the user listed the contract in 'gas_reports' (the foundry.toml field) a
            // report for the contract is generated even if it's listed in the ignore
            // list. This is addressed this way because getting a report you don't expect is
            // preferable than not getting one you expect. A warning is printed to stderr
            // indicating the "double listing".
            if self.report_for.contains(&contract_name) && self.ignore.contains(&contract_name) {
                eprintln!(
                    "{}: {} is listed in both 'gas_reports' and 'gas_reports_ignore'.",
                    yansi::Paint::yellow("warning").bold(),
                    contract_name
                );
            }
            let report_contract = (!self.ignore.contains(&contract_name) &&
                self.report_for.contains(&"*".to_string())) ||
                (!self.ignore.contains(&contract_name) && self.report_for.is_empty()) ||
                (self.report_for.contains(&contract_name));
            if report_contract {
                let contract_report = self.contracts.entry(name.to_string()).or_default();

                match &trace.data {
                    RawOrDecodedCall::Raw(bytes) if trace.created() => {
                        contract_report.gas = U256::from(trace.gas_cost);
                        contract_report.size = U256::from(bytes.len());
                    }
                    // TODO: More robust test contract filtering
                    RawOrDecodedCall::Decoded(func, sig, _)
                        if !func.is_test() && !func.is_setup() =>
                    {
                        let function_report = contract_report
                            .functions
                            .entry(func.clone())
                            .or_default()
                            .entry(sig.clone())
                            .or_default();
                        function_report.calls.push(U256::from(trace.gas_cost));
                    }
                    _ => (),
                }
            }
        }

        node.children.iter().for_each(|index| {
            self.analyze_node(*index, arena);
        });
    }

    #[must_use]
    pub fn finalize(mut self) -> Self {
        self.contracts.iter_mut().for_each(|(_, contract)| {
            contract.functions.iter_mut().for_each(|(_, sigs)| {
                sigs.iter_mut().for_each(|(_, func)| {
                    func.calls.sort_unstable();
                    func.min = func.calls.first().copied().unwrap_or_default();
                    func.max = func.calls.last().copied().unwrap_or_default();
                    func.mean = calc::mean(&func.calls);
                    func.median = U256::from(calc::median_sorted(func.calls.as_slice()));
                });
            });
        });
        self
    }
}

impl Display for GasReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        for (name, contract) in self.contracts.iter() {
            if contract.functions.is_empty() {
                continue
            }

            let mut table = Table::new();
            table.load_preset(ASCII_MARKDOWN);
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
            contract.functions.iter().for_each(|(fname, sigs)| {
                sigs.iter().for_each(|(sig, function)| {
                    // show function signature if overloaded else name
                    let fn_display =
                        if sigs.len() == 1 { fname.clone() } else { sig.replace(':', "") };

                    table.add_row(vec![
                        Cell::new(fn_display).add_attribute(Attribute::Bold),
                        Cell::new(function.min.to_string()).fg(Color::Green),
                        Cell::new(function.mean.to_string()).fg(Color::Yellow),
                        Cell::new(function.median.to_string()).fg(Color::Yellow),
                        Cell::new(function.max.to_string()).fg(Color::Red),
                        Cell::new(function.calls.len().to_string()),
                    ]);
                })
            });
            writeln!(f, "{table}")?;
            writeln!(f, "\n")?;
        }
        Ok(())
    }
}
