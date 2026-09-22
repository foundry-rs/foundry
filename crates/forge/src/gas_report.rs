//! Gas reports.

use crate::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    traces::{CallTraceArena, CallTraceDecoder, CallTraceNode, DecodedCallData},
};
use alloy_primitives::{Address, map::HashSet};
use comfy_table::{
    Cell, CellAlignment, Color, Table,
    presets::{ASCII_FULL, ASCII_MARKDOWN},
};
use foundry_common::{TestFunctionExt, calc, get_contract_name, shell};
use foundry_evm::traces::CallKind;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt::Display};

/// Represents the gas report for a set of contracts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GasReport {
    /// Whether to report any contracts.
    report_any: bool,
    /// Contracts to generate the report for.
    report_for: HashSet<String>,
    /// Contracts to ignore when generating the report.
    ignore: HashSet<String>,
    /// Whether to include gas reports for tests.
    include_tests: bool,
    /// Additional network-specific cheatcode addresses omitted from reports.
    #[serde(skip)]
    extra_cheatcode_addresses: HashSet<Address>,
    /// All contracts that were analyzed grouped by their identifier
    /// ``test/Counter.t.sol:CounterTest
    pub contracts: BTreeMap<String, ContractInfo>,
}

impl GasReport {
    pub fn new(
        report_for: impl IntoIterator<Item = String>,
        ignore: impl IntoIterator<Item = String>,
        include_tests: bool,
        extra_cheatcode_addresses: impl IntoIterator<Item = Address>,
    ) -> Self {
        let report_for = report_for.into_iter().collect::<HashSet<_>>();
        let report_any = report_for.is_empty() || report_for.contains("*");
        Self {
            report_any,
            report_for,
            ignore: ignore.into_iter().collect(),
            include_tests,
            extra_cheatcode_addresses: extra_cheatcode_addresses.into_iter().collect(),
            contracts: BTreeMap::new(),
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

    fn is_internal_address(&self, address: Address) -> bool {
        address == CHEATCODE_ADDRESS
            || address == HARDHAT_CONSOLE_ADDRESS
            || self.extra_cheatcode_addresses.contains(&address)
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
        if self.is_internal_address(trace.address) {
            return;
        }
        let Some(name) = decoder.contracts.get(&trace.address) else { return };
        let contract_name = get_contract_name(name);
        if !self.should_report(contract_name) {
            return;
        }

        let contract_info = self.contracts.entry(name.clone()).or_default();
        let is_create_call = trace.kind.is_any_create();
        if is_create_call {
            trace!(contract_name, "adding create size info");
            contract_info.size = trace.data.len();
        }

        // Only include top-level calls which account for calldata and base (21.000) cost.
        // Only include Calls and Creates as only these calls are isolated in inspector.
        if trace.depth > 1 && (trace.kind == CallKind::Call || is_create_call) {
            return;
        }

        if is_create_call {
            trace!(contract_name, "adding create gas info");
            contract_info.gas = trace.gas_used;
        } else if let Some(DecodedCallData { signature, .. }) =
            decoder.decode_function(trace).await.call_data
        {
            let name = signature.split('(').next().unwrap();
            // Ignore any test/setup functions.
            if self.include_tests || !name.test_function_kind().is_known() {
                trace!(contract_name, signature, "adding gas info");
                contract_info
                    .functions
                    .entry(name.to_string())
                    .or_default()
                    .entry(signature)
                    .or_default()
                    .frames
                    .push(trace.gas_used);
            }
        }
    }

    /// Finalizes the gas report by calculating the min, max, mean, and median for each function.
    #[must_use]
    pub fn finalize(mut self) -> Self {
        trace!("finalizing gas report");
        for func in self
            .contracts
            .values_mut()
            .flat_map(|c| c.functions.values_mut().flat_map(|s| s.values_mut()))
        {
            func.frames.sort_unstable();
            func.min = func.frames.first().copied().unwrap_or_default();
            func.max = func.frames.last().copied().unwrap_or_default();
            func.mean = calc::mean(&func.frames);
            func.median = calc::median_sorted(&func.frames);
            func.calls = func.frames.len() as u64;
        }
        self
    }

    /// Contracts with at least one recorded function call.
    fn reported_contracts(&self) -> impl Iterator<Item = (&String, &ContractInfo)> {
        self.contracts.iter().filter(|(name, contract)| {
            if contract.functions.is_empty() {
                trace!(name, "gas report contract without functions");
            }
            !contract.functions.is_empty()
        })
    }
}

impl Display for GasReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        if shell::is_json() {
            return writeln!(f, "{}", self.format_json_output());
        }
        for (name, contract) in self.reported_contracts() {
            writeln!(f, "\n{}", format_table_output(contract, name))?;
        }
        Ok(())
    }
}

impl GasReport {
    fn format_json_output(&self) -> String {
        let contracts = self
            .reported_contracts()
            .map(|(name, contract)| {
                let functions = contract
                    .functions
                    .values()
                    .flat_map(|sigs| sigs.iter().map(|(sig, info)| (sig.replace(':', ""), info)))
                    .collect::<BTreeMap<_, _>>();
                json!({
                    "contract": name,
                    "deployment": { "gas": contract.gas, "size": contract.size },
                    "functions": functions,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&contracts).unwrap()
    }
}

fn format_table_output(contract: &ContractInfo, name: &str) -> Table {
    let num = |value: &dyn Display, color: Option<Color>| {
        let cell = Cell::new(value.to_string()).set_alignment(CellAlignment::Right);
        match color {
            Some(color) => cell.fg(color),
            None => cell,
        }
    };

    let mut table = Table::new();
    if shell::is_markdown() {
        table.load_style(ASCII_MARKDOWN);
    } else {
        table.load_style(ASCII_FULL.with_rounded_corners());
    }
    table.set_header(vec![Cell::new(format!("{name} Contract")).fg(Color::Magenta)]);
    table.add_row(vec![
        Cell::new("Deployment Cost").fg(Color::Cyan),
        Cell::new("Deployment Size").fg(Color::Cyan),
    ]);
    table.add_row(vec![num(&contract.gas, None), num(&contract.size, None)]);
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
    for (fname, sigs) in &contract.functions {
        for (sig, gas_info) in sigs {
            // Show function signature if overloaded else display function name.
            let display_name = if sigs.len() == 1 { fname.clone() } else { sig.replace(':', "") };
            table.add_row(vec![
                Cell::new(display_name),
                num(&gas_info.min, Some(Color::Green)),
                num(&gas_info.mean, Some(Color::Yellow)),
                num(&gas_info.median, Some(Color::Yellow)),
                num(&gas_info.max, Some(Color::Red)),
                num(&gas_info.calls, None),
            ]);
        }
    }
    table
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm::constants::MONAD_CHEATCODE_ADDRESS;

    #[test]
    fn network_cheatcode_addresses_are_opt_in() {
        let ethereum = GasReport::new([], [], false, []);
        assert!(!ethereum.is_internal_address(MONAD_CHEATCODE_ADDRESS));

        let monad = GasReport::new([], [], false, [MONAD_CHEATCODE_ADDRESS]);
        assert!(monad.is_internal_address(MONAD_CHEATCODE_ADDRESS));
    }
}
