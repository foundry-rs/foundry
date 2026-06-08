//! Debugger implementation.

use crate::{DebugNode, DebuggerBuilder, ExitReason, tui::TUI};
use alloy_primitives::map::AddressHashMap;
use eyre::Result;
use foundry_evm_core::Breakpoints;
use foundry_evm_traces::debug::ContractSources;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebuggerStats {
    /// Total gas used by the traces passed to the debugger.
    pub total_gas_used: u64,
    /// Number of subcalls in the traces passed to the debugger.
    pub subcalls: usize,
}

impl DebuggerStats {
    #[cfg(test)]
    pub(crate) const fn from_debug_arena(debug_arena: &[DebugNode]) -> Self {
        Self { subcalls: debug_arena.len().saturating_sub(1), total_gas_used: 0 }
    }
}

pub struct DebuggerContext {
    pub debug_arena: Vec<DebugNode>,
    pub stats: DebuggerStats,
    pub identified_contracts: AddressHashMap<String>,
    /// Source map of contract sources
    pub contracts_sources: ContractSources,
    pub breakpoints: Breakpoints,
}

pub struct Debugger {
    context: DebuggerContext,
}

impl Debugger {
    /// Creates a new debugger builder.
    #[inline]
    pub fn builder() -> DebuggerBuilder {
        DebuggerBuilder::new()
    }

    /// Creates a new debugger.
    pub const fn new(
        debug_arena: Vec<DebugNode>,
        identified_contracts: AddressHashMap<String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
    ) -> Self {
        let stats =
            DebuggerStats { subcalls: debug_arena.len().saturating_sub(1), total_gas_used: 0 };
        Self::new_with_stats(
            debug_arena,
            stats,
            identified_contracts,
            contracts_sources,
            breakpoints,
        )
    }

    pub(crate) const fn new_with_stats(
        debug_arena: Vec<DebugNode>,
        stats: DebuggerStats,
        identified_contracts: AddressHashMap<String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
    ) -> Self {
        Self {
            context: DebuggerContext {
                debug_arena,
                stats,
                identified_contracts,
                contracts_sources,
                breakpoints,
            },
        }
    }

    /// Starts the debugger TUI. Terminates the current process on failure or user exit.
    pub fn run_tui_exit(mut self) -> ! {
        let code = match self.try_run_tui() {
            Ok(ExitReason::CharExit) => 0,
            Err(e) => {
                let _ = sh_eprintln!("{e}");
                1
            }
        };
        std::process::exit(code)
    }

    /// Starts the debugger TUI.
    pub fn try_run_tui(&mut self) -> Result<ExitReason> {
        eyre::ensure!(!self.context.debug_arena.is_empty(), "debug arena is empty");

        let mut tui = TUI::new(&mut self.context);
        tui.try_run()
    }

    /// Dumps debugger data to file.
    pub fn dump_to_file(&mut self, path: &Path) -> Result<()> {
        eyre::ensure!(!self.context.debug_arena.is_empty(), "debug arena is empty");
        crate::dump::dump(path, &self.context)
    }
}
