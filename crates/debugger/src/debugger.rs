//! Debugger implementation.

use crate::{DebugNode, DebuggerBuilder, ExitReason, tui::TUI};
use alloy_primitives::map::AddressHashMap;
use clap::ValueEnum;
use eyre::Result;
use foundry_common::slot_identifier::SlotIdentifier;
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
    pub(crate) slot_identifiers: Option<AddressHashMap<SlotIdentifier>>,
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
                slot_identifiers: None,
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
        slot_identifiers: AddressHashMap<SlotIdentifier>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
        layout: DebuggerLayout,
    ) -> Self {
        Self {
            context: DebuggerContext {
                debug_arena,
                stats: Some(stats),
                identified_contracts,
                slot_identifiers: Some(slot_identifiers),
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use foundry_common::ContractsByArtifactBuilder;
    use foundry_compilers::{
        ArtifactId,
        artifacts::{CompactContractBytecodeCow, StorageLayout},
    };
    use foundry_evm_traces::CallTraceDecoder;
    use std::borrow::Cow;

    fn artifact_id(name: &str, profile: &str) -> ArtifactId {
        ArtifactId {
            path: format!("out/{profile}/{name}.json").into(),
            name: name.to_string(),
            source: format!("src/{name}.sol").into(),
            version: "0.8.30".parse().unwrap(),
            build_id: profile.to_string(),
            profile: profile.to_string(),
        }
    }

    #[test]
    fn builder_skips_ambiguous_storage_layout_matches() {
        let ids = [
            artifact_id("Ambiguous", "default"),
            artifact_id("Ambiguous", "optimized"),
            artifact_id("Unique", "default"),
        ];
        let known_contracts = ContractsByArtifactBuilder::new(ids.iter().cloned().map(|id| {
            (
                id,
                CompactContractBytecodeCow {
                    abi: Some(Cow::Owned(Default::default())),
                    ..Default::default()
                },
            )
        }))
        .with_storage_layouts(ids.iter().cloned().map(|id| (id, StorageLayout::default())))
        .build();
        let ambiguous_address = Address::repeat_byte(0x11);
        let unique_address = Address::repeat_byte(0x22);
        let mut decoder = CallTraceDecoder::default();
        decoder.contracts.insert(ambiguous_address, ids[0].identifier());
        decoder.contracts.insert(unique_address, ids[2].identifier());

        let debugger =
            Debugger::builder().decoder(&decoder).known_contracts(&known_contracts).build();
        let slot_identifiers = debugger.context.slot_identifiers.as_ref().unwrap();

        assert!(!slot_identifiers.contains_key(&ambiguous_address));
        assert!(slot_identifiers.contains_key(&unique_address));
    }
}
