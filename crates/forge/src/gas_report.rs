//! Gas reports.

use crate::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    hashbrown::HashSet,
    traces::{CallTraceArena, TraceCallData, TraceKind},
};
use alloy_primitives::U256;
use comfy_table::{presets::ASCII_MARKDOWN, *};
use foundry_common::{calc, TestFunctionExt};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Display};

/// Represents the gas report for a set of contracts.
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct GasReport {
    /// Whether to report any contracts.
    report_any: bool,
    /// Contracts to generate the report for.
    report_for: HashSet<String>,
    /// Contracts to ignore when generating the report.
    ignore: HashSet<String>,
    /// All contracts that were analyzed grouped by their identifier
    /// ``test/Counter.t.sol:CounterTest
    contracts: BTreeMap<String, ContractInfo>,
}

impl GasReport {
    pub fn new(
        report_for: impl IntoIterator<Item = String>,
        ignore: impl IntoIterator<Item = String>,
    ) -> Self {
        let report_for = report_for.into_iter().collect::<HashSet<_>>();
        let ignore = ignore.into_iter().collect::<HashSet<_>>();
        let report_any = report_for.is_empty() || report_for.contains("*");
        Self { report_any, report_for, ignore, ..Default::default() }
    }

    /// Whether the given contract should be reported.
    fn should_report(&self, contract_name: &str) -> bool {
        if self.ignore.contains(contract_name) {
            // could be specified in both ignore and report_for
            return self.report_for.contains(contract_name)
        }
        self.report_any || self.report_for.contains(contract_name)
    }

    /// Analyzes the given traces and generates a gas report.
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
            let contract_name = name.rsplit(':').next().unwrap_or(name.as_str());
            // If the user listed the contract in 'gas_reports' (the foundry.toml field) a
            // report for the contract is generated even if it's listed in the ignore
            // list. This is addressed this way because getting a report you don't expect is
            // preferable than not getting one you expect. A warning is printed to stderr
            // indicating the "double listing".
            if self.report_for.contains(contract_name) && self.ignore.contains(contract_name) {
                let _ = sh_warn!(
                    "{contract_name} is listed in both 'gas_reports' and 'gas_reports_ignore'"
                );
            }

            if self.should_report(contract_name) {
                let contract_info = self.contracts.entry(name.to_string()).or_default();

                match &trace.data {
                    TraceCallData::Raw(bytes) => {
                        if trace.created() {
                            contract_info.gas = U256::from(trace.gas_cost);
                            contract_info.size = U256::from(bytes.len());
                        }
                    }
                    TraceCallData::Decoded { signature, .. } => {
                        let name = signature.split('(').next().unwrap();
                        // ignore any test/setup functions
                        let should_include =
                            !(name.is_test() || name.is_invariant_test() || name.is_setup());
                        if should_include {
                            let gas_info = contract_info
                                .functions
                                .entry(name.into())
                                .or_default()
                                .entry(signature.clone())
                                .or_default();
                            gas_info.calls.push(U256::from(trace.gas_cost));
                        }
                    }
                }
            }
        }

        node.children.iter().for_each(|index| {
            self.analyze_node(*index, arena);
        });
    }

    /// Finalizes the gas report by calculating the min, max, mean, and median for each function.
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
            table.set_header([Cell::new(format!("{name} contract"))
                .add_attribute(Attribute::Bold)
                .fg(Color::Green)]);
            table.add_row([
                Cell::new("Deployment Cost").add_attribute(Attribute::Bold).fg(Color::Cyan),
                Cell::new("Deployment Size").add_attribute(Attribute::Bold).fg(Color::Cyan),
            ]);
            table.add_row([contract.gas.to_string(), contract.size.to_string()]);

            table.add_row([
                Cell::new("Function Name").add_attribute(Attribute::Bold).fg(Color::Magenta),
                Cell::new("min").add_attribute(Attribute::Bold).fg(Color::Green),
                Cell::new("avg").add_attribute(Attribute::Bold).fg(Color::Yellow),
                Cell::new("median").add_attribute(Attribute::Bold).fg(Color::Yellow),
                Cell::new("max").add_attribute(Attribute::Bold).fg(Color::Red),
                Cell::new("# calls").add_attribute(Attribute::Bold),
            ]);
            contract.functions.iter().for_each(|(fname, sigs)| {
                sigs.iter().for_each(|(sig, gas_info)| {
                    // show function signature if overloaded else name
                    let fn_display =
                        if sigs.len() == 1 { fname.clone() } else { sig.replace(':', "") };

                    table.add_row([
                        Cell::new(fn_display).add_attribute(Attribute::Bold),
                        Cell::new(gas_info.min.to_string()).fg(Color::Green),
                        Cell::new(gas_info.mean.to_string()).fg(Color::Yellow),
                        Cell::new(gas_info.median.to_string()).fg(Color::Yellow),
                        Cell::new(gas_info.max.to_string()).fg(Color::Red),
                        Cell::new(gas_info.calls.len().to_string()),
                    ]);
                })
            });
            writeln!(f, "{table}")?;
            writeln!(f, "\n")?;
        }
        Ok(())
    }
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
