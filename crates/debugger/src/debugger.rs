//! Debugger implementation.

use crate::{tui::TUI, DebugNode, DebuggerBuilder, ExitReason, FileDumper};
use alloy_primitives::map::AddressHashMap;
use eyre::Result;
use foundry_common::evm::Breakpoints;
use foundry_evm_traces::debug::ContractSources;
use std::path::PathBuf;

pub struct DebuggerContext {
    pub debug_arena: Vec<DebugNode>,
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
    pub fn new(
        debug_arena: Vec<DebugNode>,
        identified_contracts: AddressHashMap<String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
    ) -> Self {
        Self {
            context: DebuggerContext {
                debug_arena,
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
                println!("{e}");
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
    pub fn dump_to_file(&mut self, path: &PathBuf) -> Result<()> {
        eyre::ensure!(!self.context.debug_arena.is_empty(), "debug arena is empty");

        let mut file_dumper = FileDumper::new(path, &mut self.context);
        file_dumper.run()
    }
}
