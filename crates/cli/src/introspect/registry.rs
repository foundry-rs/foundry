//! Per-binary registry of stable command metadata.
//!
//! The registry overlays metadata that cannot be expressed via clap
//! attributes onto the command tree: the stable `command_id`, capability
//! flags, and any command-specific exit codes.
//!
//! Each binary owns and ships its own [`CommandRegistry`]. Entries are keyed
//! by the **clap path** (e.g. `["forge", "build"]`). When no entry is found
//! for a command, the [`build_document`](super::build_document) helper fills
//! in safe defaults: a derived `command_id` (path joined by `.`) and no
//! machine-mode contract.
//!
//! Only commands intended to be referenced by stable identifiers need to be
//! registered explicitly. Once a command is registered, its `command_id` is
//! considered frozen and CI uniqueness checks ensure it cannot collide.

use super::document::{OutputMode, SideEffects};

/// Per-leaf metadata that overlays the clap-derived defaults.
///
/// All fields are `&'static`-friendly so each binary can declare its
/// registry as a `static` slice without heap allocation. The serialized
/// [`Capabilities`](super::Capabilities) / [`ExitCodeInfo`](super::ExitCodeInfo)
/// types use owned `String`s; conversion happens in
/// [`build_document`](super::build_document).
#[derive(Clone, Copy, Debug)]
pub struct CommandMeta {
    /// Stable machine identifier, e.g. `"forge.build"`.
    ///
    /// When set, this overrides the path-derived default.
    pub command_id: Option<&'static str>,
    /// Capabilities reported for agent consumers.
    pub capabilities: CapabilityMeta,
    /// Command-specific exit codes, in addition to the global table.
    pub exit_codes: &'static [ExitCodeMeta],
}

impl CommandMeta {
    /// Const-constructible default suitable for use in `static` registries.
    pub const DEFAULT: Self =
        Self { command_id: None, capabilities: CapabilityMeta::NONE, exit_codes: &[] };
}

impl Default for CommandMeta {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Const-friendly capability metadata stored in a static registry.
///
/// Mirrors [`Capabilities`](super::Capabilities) but with `&'static str` in
/// place of `String` so the whole struct can live in a `static`. Converted to
/// owned form during introspection rendering.
#[derive(Clone, Copy, Debug)]
pub struct CapabilityMeta {
    pub output_mode: OutputMode,
    pub result_schema_ref: Option<&'static str>,
    pub event_schema_ref: Option<&'static str>,
    pub session_schema_ref: Option<&'static str>,
    pub reads_stdin: bool,
    pub supports_output_path: bool,
    pub requires_project: bool,
    pub side_effects: SideEffects,
    pub long_running: bool,
    pub stateful: bool,
}

impl CapabilityMeta {
    /// Default with no machine-mode contract.
    pub const NONE: Self = Self {
        output_mode: OutputMode::None,
        result_schema_ref: None,
        event_schema_ref: None,
        session_schema_ref: None,
        reads_stdin: false,
        supports_output_path: false,
        requires_project: false,
        side_effects: SideEffects::None,
        long_running: false,
        stateful: false,
    };
}

impl Default for CapabilityMeta {
    fn default() -> Self {
        Self::NONE
    }
}

/// Const-friendly exit-code entry stored in a static registry.
///
/// Mirrors [`ExitCodeInfo`](super::ExitCodeInfo) with borrowed strings.
#[derive(Clone, Copy, Debug)]
pub struct ExitCodeMeta {
    pub code: i32,
    pub name: &'static str,
    pub description: &'static str,
}

/// A binary's command registry.
///
/// Implemented as a thin wrapper over a `&'static` slice of (path, metadata) pairs to keep the call
/// sites declarative without pulling in a hash map at startup.
#[derive(Clone, Copy, Debug)]
pub struct CommandRegistry {
    entries: &'static [RegistryEntry],
}

/// A single registry entry.
#[derive(Clone, Copy, Debug)]
pub struct RegistryEntry {
    /// Clap path components, e.g. `&["build"]` for `forge build`.
    ///
    /// The binary name (`forge`, `cast`, …) is not included; it is implicit
    /// from which binary owns the registry.
    pub path: &'static [&'static str],
    /// Metadata overlay for the command at `path`.
    pub meta: CommandMeta,
}

impl CommandRegistry {
    /// Construct a new registry from a static slice of entries.
    pub const fn new(entries: &'static [RegistryEntry]) -> Self {
        Self { entries }
    }

    /// An empty registry. Every command falls back to defaults.
    pub const EMPTY: Self = Self::new(&[]);

    /// Look up metadata for the command at `path` (clap path, excluding the
    /// binary name).
    pub fn lookup(&self, path: &[&str]) -> Option<&CommandMeta> {
        self.entries.iter().find(|e| e.path == path).map(|e| &e.meta)
    }

    /// Iterate over all entries.
    pub fn entries(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_registry() -> CommandRegistry {
        static ENTRIES: &[RegistryEntry] = &[RegistryEntry {
            path: &["build"],
            meta: CommandMeta {
                command_id: Some("forge.build"),
                capabilities: CapabilityMeta {
                    output_mode: OutputMode::Envelope,
                    result_schema_ref: Some("foundry:forge.build@v1"),
                    requires_project: true,
                    side_effects: super::super::document::SideEffects::FsWrite,
                    ..CapabilityMeta::NONE
                },
                exit_codes: &[],
            },
        }];
        CommandRegistry::new(ENTRIES)
    }

    #[test]
    fn lookup_returns_registered_meta() {
        let r = fixture_registry();
        let meta = r.lookup(&["build"]).expect("registered");
        assert_eq!(meta.command_id, Some("forge.build"));
        assert!(matches!(meta.capabilities.output_mode, OutputMode::Envelope));
    }

    #[test]
    fn lookup_returns_none_for_unregistered_path() {
        assert!(fixture_registry().lookup(&["unknown"]).is_none());
    }

    #[test]
    fn empty_registry_yields_no_entries() {
        assert_eq!(CommandRegistry::EMPTY.entries().count(), 0);
    }
}
