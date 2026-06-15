//! Debugger implementation.

use crate::{DebugNode, DebuggerBuilder, ExitReason, tui::TUI};
use alloy_primitives::map::AddressHashMap;
use clap::ValueEnum;
use eyre::Result;
use foundry_evm_core::Breakpoints;
use foundry_evm_traces::debug::ContractSources;
use std::path::Path;

/// Debugger TUI layout selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum DebuggerLayout {
    /// Select horizontal or vertical layout from the terminal size.
    #[default]
    Auto,
    /// Force the two-column debugger layout.
    Horizontal,
    /// Force the single-column debugger layout.
    Vertical,
}

impl DebuggerLayout {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Auto | Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebuggerStats {
    /// Sum of root-call gas used across every trace arena passed to the debugger.
    pub session_trace_gas_used: u64,
    /// Number of subcalls in the traces passed to the debugger.
    pub session_subcalls: usize,
}

pub struct DebuggerContext {
    pub debug_arena: Vec<DebugNode>,
    pub stats: Option<DebuggerStats>,
    pub identified_contracts: AddressHashMap<String>,
    /// Source map of contract sources
    pub contracts_sources: ContractSources,
    pub breakpoints: Breakpoints,
    pub layout: DebuggerLayout,
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
        Self {
            context: DebuggerContext {
                debug_arena,
                stats: None,
                identified_contracts,
                contracts_sources,
                breakpoints,
                layout: DebuggerLayout::Auto,
            },
        }
    }

    pub(crate) const fn new_with_stats(
        debug_arena: Vec<DebugNode>,
        stats: DebuggerStats,
        identified_contracts: AddressHashMap<String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
        layout: DebuggerLayout,
    ) -> Self {
        Self {
            context: DebuggerContext {
                debug_arena,
                stats: Some(stats),
                identified_contracts,
                contracts_sources,
                breakpoints,
                layout,
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
