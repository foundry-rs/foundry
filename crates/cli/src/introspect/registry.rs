//! Per-binary registry of stable command metadata.
//!
//! The registry overlays metadata that cannot be expressed via clap
//! attributes onto the command tree: the stable `command_id`, capability
//! flags, and any command-specific exit codes.
//!
//! Each binary owns and ships its own [`CommandRegistry`]. Entries are keyed
//! by the **clap path excluding the binary name** (e.g. `["build"]` for
//! `forge build`); the binary name is implicit from the owning binary. When
//! no entry is found for a command, the
//! [`build_document`](super::build_document) helper fills in safe defaults: a
//! derived `command_id` (path joined by `.`) marked `command_id_stable=false`
//! and `Capabilities::NONE` marked `capabilities_declared=false`.
//!
//! Only commands intended to be referenced by stable identifiers need to be
//! registered explicitly. Once a command is registered, its `command_id` is
//! considered frozen and CI uniqueness checks ensure it cannot collide.

use super::document::{Capabilities, ExitCodeInfo};

/// Per-leaf metadata that overlays the clap-derived defaults.
#[derive(Clone, Debug)]
pub struct CommandMeta {
    /// Stable machine identifier, e.g. `"forge.build"`.
    ///
    /// When set, this overrides the path-derived default.
    pub command_id: Option<&'static str>,
    /// Capabilities reported for agent consumers.
    pub capabilities: Capabilities,
    /// Set to `true` when `capabilities` is intentionally authored; partial
    /// entries that pin only `command_id` or `exit_codes` must leave this `false`.
    pub capabilities_declared: bool,
    /// Command-specific exit codes, in addition to the global table.
    pub exit_codes: &'static [ExitCodeInfo],
}

impl CommandMeta {
    /// Const-constructible default suitable for use in `static` registries.
    pub const DEFAULT: Self = Self {
        command_id: None,
        capabilities: Capabilities::NONE,
        capabilities_declared: false,
        exit_codes: &[],
    };
}

impl Default for CommandMeta {
    fn default() -> Self {
        Self::DEFAULT
    }
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
#[derive(Clone, Debug)]
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
    use crate::introspect::document::OutputMode;
    use std::borrow::Cow;

    fn fixture_registry() -> CommandRegistry {
        static ENTRIES: &[RegistryEntry] = &[RegistryEntry {
            path: &["build"],
            meta: CommandMeta {
                command_id: Some("forge.build"),
                capabilities: Capabilities {
                    output_mode: OutputMode::Envelope,
                    result_schema_ref: None,
                    event_schema_ref: None,
                    session_schema_ref: None,
                    reads_stdin: false,
                    supports_output_path: false,
                    requires_project: true,
                    side_effects: super::super::document::SideEffects::FsWrite,
                    long_running: false,
                    stateful: false,
                },
                capabilities_declared: true,
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

    /// The root/default invocation is keyed by the empty path; verify that
    /// `lookup(&[])` finds an entry registered at `path: &[]`.
    #[test]
    fn lookup_supports_empty_path_for_root_default() {
        static ENTRIES: &[RegistryEntry] = &[RegistryEntry {
            path: &[],
            meta: CommandMeta {
                command_id: Some("anvil.start"),
                capabilities: Capabilities::NONE,
                capabilities_declared: false,
                exit_codes: &[],
            },
        }];
        let registry = CommandRegistry::new(ENTRIES);
        let meta = registry.lookup(&[]).expect("root/default entry");
        assert_eq!(meta.command_id, Some("anvil.start"));
    }

    /// A registry with real strings (schema refs, exit-code names) must be
    /// authorable in a plain `static` without lazy allocation.
    #[test]
    fn static_registry_supports_real_strings() {
        static EXITS: &[ExitCodeInfo] = &[ExitCodeInfo {
            code: 2,
            name: Cow::Borrowed("TestFailure"),
            description: Cow::Borrowed("at least one test failed"),
        }];
        static ENTRIES: &[RegistryEntry] = &[RegistryEntry {
            path: &["test"],
            meta: CommandMeta {
                command_id: Some("forge.test"),
                capabilities: Capabilities {
                    output_mode: OutputMode::Envelope,
                    result_schema_ref: Some(Cow::Borrowed("foundry:test-result@v1")),
                    event_schema_ref: None,
                    session_schema_ref: None,
                    reads_stdin: false,
                    supports_output_path: false,
                    requires_project: true,
                    side_effects: super::super::document::SideEffects::None,
                    long_running: false,
                    stateful: false,
                },
                capabilities_declared: true,
                exit_codes: EXITS,
            },
        }];
        let registry = CommandRegistry::new(ENTRIES);

        let meta = registry.lookup(&["test"]).unwrap();
        assert_eq!(meta.capabilities.result_schema_ref.as_deref(), Some("foundry:test-result@v1"));
        assert_eq!(meta.exit_codes[0].name, "TestFailure");
    }
}
