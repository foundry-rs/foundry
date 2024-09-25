//! Gas reports.

use crate::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    traces::{CallTraceArena, CallTraceDecoder, CallTraceNode, DecodedCallData},
};
use comfy_table::{presets::ASCII_MARKDOWN, *};
use foundry_common::{calc, TestFunctionExt};
use foundry_evm::traces::CallKind;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt::Display,
};
use yansi::Paint;

/// Represents the gas report for a set of contracts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GasReport {
    /// Whether to report any contracts.
    report_any: bool,
    /// Contracts to generate the report for.
    report_for: HashSet<String>,
    /// Contracts to ignore when generating the report.
    ignore: HashSet<String>,
    /// All contracts that were analyzed grouped by their identifier
    /// ``test/Counter.t.sol:CounterTest
    pub contracts: BTreeMap<String, ContractInfo>,
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
                eprintln!(
                    "{}: {} is listed in both 'gas_reports' and 'gas_reports_ignore'.",
                    "warning".yellow().bold(),
                    contract_name
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

        // Only include top-level calls which account for calldata and base (21.000) cost.
        // Only include Calls and Creates as only these calls are isolated in inspector.
        if trace.depth > 1 &&
            (trace.kind == CallKind::Call ||
                trace.kind == CallKind::Create ||
                trace.kind == CallKind::Create2 ||
                trace.kind == CallKind::EOFCreate)
        {
            return;
        }

        let Some(name) = decoder.contracts.get(&node.trace.address) else { return };
        let contract_name = name.rsplit(':').next().unwrap_or(name);

        if !self.should_report(contract_name) {
            return;
        }

        let decoded = || decoder.decode_function(&node.trace);

        let contract_info = self.contracts.entry(name.to_string()).or_default();
        if trace.kind.is_any_create() {
            trace!(contract_name, "adding create gas info");
            contract_info.gas = trace.gas_used;
            contract_info.size = trace.data.len();
        } else if let Some(DecodedCallData { signature, .. }) = decoded().await.call_data {
            let name = signature.split('(').next().unwrap();
            // ignore any test/setup functions
            if !name.test_function_kind().is_known() {
                trace!(contract_name, signature, "adding gas info");
                let gas_info = contract_info
                    .functions
                    .entry(name.to_string())
                    .or_default()
                    .entry(signature.clone())
                    .or_default();
                gas_info.calls.push(trace.gas_used);
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
                    func.calls.sort_unstable();
                    func.min = func.calls.first().copied().unwrap_or_default();
                    func.max = func.calls.last().copied().unwrap_or_default();
                    func.mean = calc::mean(&func.calls);
                    func.median = calc::median_sorted(&func.calls);
                }
            }
        }
        self
    }
}

impl Display for GasReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        for (name, contract) in &self.contracts {
            if contract.functions.is_empty() {
                trace!(name, "gas report contract without functions");
                continue;
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContractInfo {
    pub gas: u64,
    pub size: usize,
    /// Function name -> Function signature -> GasInfo
    pub functions: BTreeMap<String, BTreeMap<String, GasInfo>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GasInfo {
    pub calls: Vec<u64>,
    pub min: u64,
    pub mean: u64,
    pub median: u64,
    pub max: u64,
}
