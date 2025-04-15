//! Gas reports.

use crate::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    traces::{CallTraceArena, CallTraceDecoder, CallTraceNode, DecodedCallData},
};
use alloy_primitives::map::HashSet;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, Cell, Color, Table};
use foundry_common::{
    calc,
    reports::{report_kind, ReportKind},
    TestFunctionExt,
};
use foundry_evm::traces::CallKind;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt::Display};

/// Represents the gas report for a set of contracts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GasReport {
    /// Whether to report any contracts.
    report_any: bool,
    /// What kind of report to generate.
    report_kind: ReportKind,
    /// Contracts to generate the report for.
    report_for: HashSet<String>,
    /// Contracts to ignore when generating the report.
    ignore: HashSet<String>,
    /// Whether to include gas reports for tests.
    include_tests: bool,
    /// All contracts that were analyzed grouped by their identifier
    /// ``test/Counter.t.sol:CounterTest
    pub contracts: BTreeMap<String, ContractInfo>,
}

impl GasReport {
    pub fn new(
        report_for: impl IntoIterator<Item = String>,
        ignore: impl IntoIterator<Item = String>,
        include_tests: bool,
    ) -> Self {
        let report_for = report_for.into_iter().collect::<HashSet<_>>();
        let ignore = ignore.into_iter().collect::<HashSet<_>>();
        let report_any = report_for.is_empty() || report_for.contains("*");
        Self {
            report_any,
            report_kind: report_kind(),
            report_for,
            ignore,
            include_tests,
            ..Default::default()
        }
    }

    /// Whether the given contract should be reported.
    #[instrument(level = "trace", skip(self), ret)]
    fn should_report(&self, contract_name: &str) -> bool {
        if self.ignore.contains(contract_name) {
            let contains_anyway = self.report_for.contains(contract_name);
            if contains_anyway {
                // If the user listed the contract in 'gas_reports' (the foundry.toml field) a
                // report for the contract is generated even if it's listed in the ignore
                // list. This is addressed this way because getting a report you don't expect is
                // preferable than not getting one you expect. A warning is printed to stderr
                // indicating the "double listing".
                let _ = sh_warn!(
                    "{contract_name} is listed in both 'gas_reports' and 'gas_reports_ignore'."
                );
            }
            return contains_anyway;
        }
        self.report_any || self.report_for.contains(contract_name)
    }

    /// Analyzes the given traces and generates a gas report.
    pub async fn analyze(
        &mut self,
        arenas: impl IntoIterator<Item = &CallTraceArena>,
        decoder: &CallTraceDecoder,
    ) {
        for node in arenas.into_iter().flat_map(|arena| arena.nodes()) {
            self.analyze_node(node, decoder).await;
        }
    }

    async fn analyze_node(&mut self, node: &CallTraceNode, decoder: &CallTraceDecoder) {
        let trace = &node.trace;

        if trace.address == CHEATCODE_ADDRESS || trace.address == HARDHAT_CONSOLE_ADDRESS {
            return;
        }

        let Some(name) = decoder.contracts.get(&node.trace.address) else { return };
        let contract_name = name.rsplit(':').next().unwrap_or(name);

        if !self.should_report(contract_name) {
            return;
        }
        let contract_info = self.contracts.entry(name.to_string()).or_default();
        let is_create_call = trace.kind.is_any_create();

        // Record contract deployment size.
        if is_create_call {
            trace!(contract_name, "adding create size info");
            contract_info.size = trace.data.len();
        }

        // Only include top-level calls which account for calldata and base (21.000) cost.
        // Only include Calls and Creates as only these calls are isolated in inspector.
        if trace.depth > 1 && (trace.kind == CallKind::Call || is_create_call) {
            return;
        }

        let decoded = || decoder.decode_function(&node.trace);

        if is_create_call {
            trace!(contract_name, "adding create gas info");
            contract_info.gas = trace.gas_used;
        } else if let Some(DecodedCallData { signature, .. }) = decoded().await.call_data {
            let name = signature.split('(').next().unwrap();
            // ignore any test/setup functions
            if self.include_tests || !name.test_function_kind().is_known() {
                trace!(contract_name, signature, "adding gas info");
                let gas_info = contract_info
                    .functions
                    .entry(name.to_string())
                    .or_default()
                    .entry(signature.clone())
                    .or_default();
                gas_info.frames.push(trace.gas_used);
            }
        }
    }

    /// Finalizes the gas report by calculating the min, max, mean, and median for each function.
    #[must_use]
    pub fn finalize(mut self) -> Self {
        trace!("finalizing gas report");
        for contract in self.contracts.values_mut() {
            for sigs in contract.functions.values_mut() {
                for func in sigs.values_mut() {
                    func.frames.sort_unstable();
                    func.min = func.frames.first().copied().unwrap_or_default();
                    func.max = func.frames.last().copied().unwrap_or_default();
                    func.mean = calc::mean(&func.frames);
                    func.median = calc::median_sorted(&func.frames);
                    func.calls = func.frames.len() as u64;
                }
            }
        }
        self
    }
}

impl Display for GasReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.report_kind {
            ReportKind::Text => {
                for (name, contract) in &self.contracts {
                    if contract.functions.is_empty() {
                        trace!(name, "gas report contract without functions");
                        continue;
                    }

                    let table = self.format_table_output(contract, name);
                    writeln!(f, "\n{table}")?;
                }
            }
            ReportKind::JSON => {
                writeln!(f, "{}", &self.format_json_output())?;
            }
        }

        Ok(())
    }
}

impl GasReport {
    fn format_json_output(&self) -> String {
        serde_json::to_string(
            &self
                .contracts
                .iter()
                .filter_map(|(name, contract)| {
                    if contract.functions.is_empty() {
                        trace!(name, "gas report contract without functions");
                        return None;
                    }

                    let functions = contract
                        .functions
                        .iter()
                        .flat_map(|(_, sigs)| {
                            sigs.iter().map(|(sig, gas_info)| {
                                let display_name = sig.replace(':', "");
                                (display_name, gas_info)
                            })
                        })
                        .collect::<BTreeMap<_, _>>();

                    Some(json!({
                        "contract": name,
                        "deployment": {
                            "gas": contract.gas,
                            "size": contract.size,
                        },
                        "functions": functions,
                    }))
                })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn format_table_output(&self, contract: &ContractInfo, name: &str) -> Table {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);

        table.set_header(vec![Cell::new(format!("{name} Contract")).fg(Color::Magenta)]);

        table.add_row(vec![
            Cell::new("Deployment Cost").fg(Color::Cyan),
            Cell::new("Deployment Size").fg(Color::Cyan),
        ]);
        table.add_row(vec![
            Cell::new(contract.gas.to_string()),
            Cell::new(contract.size.to_string()),
        ]);

        // Add a blank row to separate deployment info from function info.
        table.add_row(vec![Cell::new("")]);

        table.add_row(vec![
            Cell::new("Function Name"),
            Cell::new("Min").fg(Color::Green),
            Cell::new("Avg").fg(Color::Yellow),
            Cell::new("Median").fg(Color::Yellow),
            Cell::new("Max").fg(Color::Red),
            Cell::new("# Calls").fg(Color::Cyan),
        ]);

        contract.functions.iter().for_each(|(fname, sigs)| {
            sigs.iter().for_each(|(sig, gas_info)| {
                // Show function signature if overloaded else display function name.
                let display_name =
                    if sigs.len() == 1 { fname.to_string() } else { sig.replace(':', "") };

                table.add_row(vec![
                    Cell::new(display_name),
                    Cell::new(gas_info.min.to_string()).fg(Color::Green),
                    Cell::new(gas_info.mean.to_string()).fg(Color::Yellow),
                    Cell::new(gas_info.median.to_string()).fg(Color::Yellow),
                    Cell::new(gas_info.max.to_string()).fg(Color::Red),
                    Cell::new(gas_info.calls.to_string()),
                ]);
            })
        });

        table
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContractInfo {
    pub gas: u64,
    pub size: usize,
    /// Function name -> Function signature -> GasInfo
    pub functions: BTreeMap<String, BTreeMap<String, GasInfo>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GasInfo {
    pub calls: u64,
    pub min: u64,
    pub mean: u64,
    pub median: u64,
    pub max: u64,

    #[serde(skip)]
    pub frames: Vec<u64>,
}
