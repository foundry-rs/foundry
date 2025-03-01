//! Solc artifact types.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![allow(ambiguous_glob_reexports)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use semver::Version;
use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

pub mod error;
pub use error::*;
pub mod ast;
pub use ast::*;
pub mod remappings;
pub use remappings::*;
pub mod bytecode;
pub use bytecode::*;
pub mod contract;
pub use contract::*;
pub mod configurable;
pub mod hh;
pub use configurable::*;
pub mod output_selection;
pub mod serde_helpers;
pub mod sourcemap;
pub mod sources;
use crate::output_selection::{ContractOutputSelection, OutputSelection};
use foundry_compilers_core::{
    error::SolcError,
    utils::{
        strip_prefix_owned, BERLIN_SOLC, BYZANTIUM_SOLC, CANCUN_SOLC, CONSTANTINOPLE_SOLC,
        ISTANBUL_SOLC, LONDON_SOLC, PARIS_SOLC, PETERSBURG_SOLC, PRAGUE_SOLC, SHANGHAI_SOLC,
    },
};
pub use serde_helpers::{deserialize_bytes, deserialize_opt_bytes};
pub use sources::*;

/// Solidity files are made up of multiple `source units`, a solidity contract is such a `source
/// unit`, therefore a solidity file can contain multiple contracts: (1-N*) relationship.
///
/// This types represents this mapping as `file name -> (contract name -> T)`, where the generic is
/// intended to represent contract specific information, like [`Contract`] itself, See [`Contracts`]
pub type FileToContractsMap<T> = BTreeMap<PathBuf, BTreeMap<String, T>>;

/// file -> (contract name -> Contract)
pub type Contracts = FileToContractsMap<Contract>;

pub const SOLIDITY: &str = "Solidity";
pub const YUL: &str = "Yul";

/// Languages supported by the Solc compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SolcLanguage {
    Solidity,
    Yul,
}

impl fmt::Display for SolcLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solidity => write!(f, "Solidity"),
            Self::Yul => write!(f, "Yul"),
        }
    }
}

/// Input type `solc` expects.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolcInput {
    pub language: SolcLanguage,
    pub sources: Sources,
    pub settings: Settings,
}

/// Default `language` field is set to `"Solidity"`.
impl Default for SolcInput {
    fn default() -> Self {
        Self {
            language: SolcLanguage::Solidity,
            sources: Sources::default(),
            settings: Settings::default(),
        }
    }
}

impl SolcInput {
    pub fn new(language: SolcLanguage, sources: Sources, mut settings: Settings) -> Self {
        if language == SolcLanguage::Yul && !settings.remappings.is_empty() {
            warn!("omitting remappings supplied for the yul sources");
            settings.remappings = vec![];
        }
        Self { language, sources, settings }
    }

    /// Builds one or two inputs from given sources set. Returns two inputs in cases when there are
    /// both Solidity and Yul sources.
    pub fn resolve_and_build(sources: Sources, settings: Settings) -> Vec<Self> {
        let mut solidity_sources = Sources::new();
        let mut yul_sources = Sources::new();

        for (file, source) in sources {
            if file.extension().is_some_and(|e| e == "yul") {
                yul_sources.insert(file, source);
            } else if file.extension().is_some_and(|e| e == "sol") {
                solidity_sources.insert(file, source);
            }
        }

        let mut res = Vec::new();

        if !solidity_sources.is_empty() {
            res.push(Self::new(SolcLanguage::Solidity, solidity_sources, settings.clone()))
        }

        if !yul_sources.is_empty() {
            res.push(Self::new(SolcLanguage::Yul, yul_sources, settings))
        }

        res
    }

    /// This will remove/adjust values in the [`SolcInput`] that are not compatible with this
    /// version
    pub fn sanitize(&mut self, version: &Version) {
        self.settings.sanitize(version, self.language);
    }

    /// Consumes the type and returns a [SolcInput::sanitized] version
    pub fn sanitized(mut self, version: &Version) -> Self {
        self.settings.sanitize(version, self.language);
        self
    }

    /// Sets the EVM version for compilation
    #[must_use]
    pub fn evm_version(mut self, version: EvmVersion) -> Self {
        self.settings.evm_version = Some(version);
        self
    }

    /// Sets the optimizer runs (default = 200)
    #[must_use]
    pub fn optimizer(mut self, runs: usize) -> Self {
        self.settings.optimizer.runs(runs);
        self
    }

    /// Sets the path of the source files to `root` adjoined to the existing path
    #[must_use]
    pub fn join_path(mut self, root: &Path) -> Self {
        self.sources = self.sources.into_iter().map(|(path, s)| (root.join(path), s)).collect();
        self
    }

    /// Removes the `base` path from all source files
    pub fn strip_prefix(&mut self, base: &Path) {
        self.sources = std::mem::take(&mut self.sources)
            .into_iter()
            .map(|(path, s)| (strip_prefix_owned(path, base), s))
            .collect();

        self.settings.strip_prefix(base);
    }

    /// The flag indicating whether the current [SolcInput] is
    /// constructed for the yul sources
    pub fn is_yul(&self) -> bool {
        self.language == SolcLanguage::Yul
    }
}

/// A `CompilerInput` representation used for verify
///
/// This type is an alternative `CompilerInput` but uses non-alphabetic ordering of the `sources`
/// and instead emits the (Path -> Source) path in the same order as the pairs in the `sources`
/// `Vec`. This is used over a map, so we can determine the order in which etherscan will display
/// the verified contracts
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StandardJsonCompilerInput {
    pub language: SolcLanguage,
    #[serde(with = "serde_helpers::tuple_vec_map")]
    pub sources: Vec<(PathBuf, Source)>,
    pub settings: Settings,
}

// === impl StandardJsonCompilerInput ===

impl StandardJsonCompilerInput {
    pub fn new(sources: Vec<(PathBuf, Source)>, settings: Settings) -> Self {
        Self { language: SolcLanguage::Solidity, sources, settings }
    }

    /// Normalizes the EVM version used in the settings to be up to the latest one
    /// supported by the provided compiler version.
    #[must_use]
    pub fn normalize_evm_version(mut self, version: &Version) -> Self {
        if let Some(evm_version) = &mut self.settings.evm_version {
            self.settings.evm_version = evm_version.normalize_version_solc(version);
        }
        self
    }
}

impl From<StandardJsonCompilerInput> for SolcInput {
    fn from(input: StandardJsonCompilerInput) -> Self {
        let StandardJsonCompilerInput { language, sources, settings } = input;
        Self { language, sources: sources.into_iter().collect(), settings }
    }
}

impl From<SolcInput> for StandardJsonCompilerInput {
    fn from(input: SolcInput) -> Self {
        let SolcInput { language, sources, settings, .. } = input;
        Self { language, sources: sources.into_iter().collect(), settings }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Stop compilation after the given stage.
    /// since 0.8.11: only "parsing" is valid here
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_after: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remappings: Vec<Remapping>,
    /// Custom Optimizer settings
    #[serde(default)]
    pub optimizer: Optimizer,
    /// Model Checker options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_checker: Option<ModelCheckerSettings>,
    /// Metadata settings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SettingsMetadata>,
    /// This field can be used to select desired outputs based
    /// on file and contract names.
    /// If this field is omitted, then the compiler loads and does type
    /// checking, but will not generate any outputs apart from errors.
    #[serde(default)]
    pub output_selection: OutputSelection,
    #[serde(
        default,
        with = "serde_helpers::display_from_str_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub evm_version: Option<EvmVersion>,
    /// Change compilation pipeline to go through the Yul intermediate representation. This is
    /// false by default.
    #[serde(rename = "viaIR", default, skip_serializing_if = "Option::is_none")]
    pub via_ir: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebuggingSettings>,
    /// Addresses of the libraries. If not all libraries are given here,
    /// it can result in unlinked objects whose output data is different.
    ///
    /// The top level key is the name of the source file where the library is used.
    /// If remappings are used, this source file should match the global path
    /// after remappings were applied.
    /// If this key is an empty string, that refers to a global level.
    #[serde(default)]
    pub libraries: Libraries,
    /// Specify EOF version to produce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eof_version: Option<EofVersion>,
}

/// Available EOF versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum EofVersion {
    V1 = 1,
}

impl Settings {
    /// Creates a new `Settings` instance with the given `output_selection`
    pub fn new(output_selection: impl Into<OutputSelection>) -> Self {
        Self { output_selection: output_selection.into(), ..Default::default() }
    }

    /// Consumes the type and returns a [Settings::sanitize] version
    pub fn sanitized(mut self, version: &Version, language: SolcLanguage) -> Self {
        self.sanitize(version, language);
        self
    }

    /// This will remove/adjust values in the settings that are not compatible with this version.
    pub fn sanitize(&mut self, version: &Version, language: SolcLanguage) {
        if *version < Version::new(0, 6, 0) {
            if let Some(meta) = &mut self.metadata {
                // introduced in <https://docs.soliditylang.org/en/v0.6.0/using-the-compiler.html#compiler-api>
                // missing in <https://docs.soliditylang.org/en/v0.5.17/using-the-compiler.html#compiler-api>
                meta.bytecode_hash = None;
            }
            // introduced in <https://docs.soliditylang.org/en/v0.6.0/using-the-compiler.html#compiler-api>
            self.debug = None;
        }

        if *version < Version::new(0, 7, 5) {
            // introduced in 0.7.5 <https://github.com/ethereum/solidity/releases/tag/v0.7.5>
            self.via_ir = None;
        }

        if *version < Version::new(0, 8, 5) {
            // introduced in 0.8.5 <https://github.com/ethereum/solidity/releases/tag/v0.8.5>
            if let Some(optimizer_details) = &mut self.optimizer.details {
                optimizer_details.inliner = None;
            }
        }

        if *version < Version::new(0, 8, 7) {
            // lower the disable version from 0.8.10 to 0.8.7, due to `divModNoSlacks`,
            // `showUnproved` and `solvers` are implemented
            // introduced in <https://github.com/ethereum/solidity/releases/tag/v0.8.7>
            self.model_checker = None;
        }

        if *version < Version::new(0, 8, 10) {
            if let Some(debug) = &mut self.debug {
                // introduced in <https://docs.soliditylang.org/en/v0.8.10/using-the-compiler.html#compiler-api>
                // <https://github.com/ethereum/solidity/releases/tag/v0.8.10>
                debug.debug_info.clear();
            }

            if let Some(model_checker) = &mut self.model_checker {
                // introduced in <https://github.com/ethereum/solidity/releases/tag/v0.8.10>
                model_checker.invariants = None;
            }
        }

        if *version < Version::new(0, 8, 18) {
            // introduced in 0.8.18 <https://github.com/ethereum/solidity/releases/tag/v0.8.18>
            if let Some(meta) = &mut self.metadata {
                meta.cbor_metadata = None;
            }

            if let Some(model_checker) = &mut self.model_checker {
                if let Some(solvers) = &mut model_checker.solvers {
                    // elf solver introduced in 0.8.18 <https://github.com/ethereum/solidity/releases/tag/v0.8.18>
                    solvers.retain(|solver| *solver != ModelCheckerSolver::Eld);
                }
            }
        }

        if *version < Version::new(0, 8, 20) {
            // introduced in 0.8.20 <https://github.com/ethereum/solidity/releases/tag/v0.8.20>
            if let Some(model_checker) = &mut self.model_checker {
                model_checker.show_proved_safe = None;
                model_checker.show_unsupported = None;
            }
        }

        if let Some(evm_version) = self.evm_version {
            self.evm_version = evm_version.normalize_version_solc(version);
        }

        match language {
            SolcLanguage::Solidity => {}
            SolcLanguage::Yul => {
                if !self.remappings.is_empty() {
                    warn!("omitting remappings supplied for the yul sources");
                }
                self.remappings = Vec::new();
            }
        }
    }

    /// Inserts a set of `ContractOutputSelection`
    pub fn push_all(&mut self, settings: impl IntoIterator<Item = ContractOutputSelection>) {
        for value in settings {
            self.push_output_selection(value)
        }
    }

    /// Inserts a set of `ContractOutputSelection`
    #[must_use]
    pub fn with_extra_output(
        mut self,
        settings: impl IntoIterator<Item = ContractOutputSelection>,
    ) -> Self {
        for value in settings {
            self.push_output_selection(value)
        }
        self
    }

    /// Inserts the value for all files and contracts
    ///
    /// ```
    /// use foundry_compilers_artifacts_solc::{output_selection::ContractOutputSelection, Settings};
    /// let mut selection = Settings::default();
    /// selection.push_output_selection(ContractOutputSelection::Metadata);
    /// ```
    pub fn push_output_selection(&mut self, value: impl ToString) {
        self.push_contract_output_selection("*", value)
    }

    /// Inserts the `key` `value` pair to the `output_selection` for all files
    ///
    /// If the `key` already exists, then the value is added to the existing list
    pub fn push_contract_output_selection(
        &mut self,
        contracts: impl Into<String>,
        value: impl ToString,
    ) {
        let value = value.to_string();
        let values = self
            .output_selection
            .as_mut()
            .entry("*".to_string())
            .or_default()
            .entry(contracts.into())
            .or_default();
        if !values.contains(&value) {
            values.push(value)
        }
    }

    /// Sets the value for all files and contracts
    pub fn set_output_selection(&mut self, values: impl IntoIterator<Item = impl ToString>) {
        self.set_contract_output_selection("*", values)
    }

    /// Sets the `key` to the `values` pair to the `output_selection` for all files
    ///
    /// This will replace the existing values for `key` if they're present
    pub fn set_contract_output_selection(
        &mut self,
        key: impl Into<String>,
        values: impl IntoIterator<Item = impl ToString>,
    ) {
        self.output_selection
            .as_mut()
            .entry("*".to_string())
            .or_default()
            .insert(key.into(), values.into_iter().map(|s| s.to_string()).collect());
    }

    /// Sets the `viaIR` value.
    #[must_use]
    pub fn set_via_ir(mut self, via_ir: bool) -> Self {
        self.via_ir = Some(via_ir);
        self
    }

    /// Enables `viaIR`.
    #[must_use]
    pub fn with_via_ir(self) -> Self {
        self.set_via_ir(true)
    }

    /// Enable `viaIR` and use the minimum optimization settings.
    ///
    /// This is useful in the following scenarios:
    /// - When compiling for test coverage, this can resolve the "stack too deep" error while still
    ///   giving a relatively accurate source mapping
    /// - When compiling for test, this can reduce the compilation time
    pub fn with_via_ir_minimum_optimization(mut self) -> Self {
        // https://github.com/foundry-rs/foundry/pull/5349
        // https://github.com/ethereum/solidity/issues/12533#issuecomment-1013073350
        self.via_ir = Some(true);
        self.optimizer.details = Some(OptimizerDetails {
            peephole: Some(false),
            inliner: Some(false),
            jumpdest_remover: Some(false),
            order_literals: Some(false),
            deduplicate: Some(false),
            cse: Some(false),
            constant_optimizer: Some(false),
            yul: Some(true), // enable yul optimizer
            yul_details: Some(YulDetails {
                stack_allocation: Some(true),
                // with only unused prunner step
                optimizer_steps: Some("u".to_string()),
            }),
            // Set to None as it is only supported for solc starting from 0.8.22.
            simple_counter_for_loop_unchecked_increment: None,
        });
        self
    }

    /// Adds `ast` to output
    #[must_use]
    pub fn with_ast(mut self) -> Self {
        let output = self.output_selection.as_mut().entry("*".to_string()).or_default();
        output.insert(String::new(), vec!["ast".to_string()]);
        self
    }

    pub fn strip_prefix(&mut self, base: &Path) {
        self.remappings.iter_mut().for_each(|r| {
            r.strip_prefix(base);
        });

        self.libraries.libs = std::mem::take(&mut self.libraries.libs)
            .into_iter()
            .map(|(file, libs)| (file.strip_prefix(base).map(Into::into).unwrap_or(file), libs))
            .collect();

        self.output_selection = OutputSelection(
            std::mem::take(&mut self.output_selection.0)
                .into_iter()
                .map(|(file, selection)| {
                    (
                        Path::new(&file)
                            .strip_prefix(base)
                            .map(|p| p.display().to_string())
                            .unwrap_or(file),
                        selection,
                    )
                })
                .collect(),
        );

        if let Some(mut model_checker) = self.model_checker.take() {
            model_checker.contracts = model_checker
                .contracts
                .into_iter()
                .map(|(path, contracts)| {
                    (
                        Path::new(&path)
                            .strip_prefix(base)
                            .map(|p| p.display().to_string())
                            .unwrap_or(path),
                        contracts,
                    )
                })
                .collect();
            self.model_checker = Some(model_checker);
        }
    }

    /// Strips `base` from all paths
    pub fn with_base_path(mut self, base: &Path) -> Self {
        self.strip_prefix(base);
        self
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stop_after: None,
            optimizer: Default::default(),
            metadata: None,
            output_selection: OutputSelection::default_output_selection(),
            evm_version: Some(EvmVersion::default()),
            via_ir: None,
            debug: None,
            libraries: Default::default(),
            remappings: Default::default(),
            model_checker: None,
            eof_version: None,
        }
        .with_ast()
    }
}

/// A wrapper type for all libraries in the form of `<file>:<lib>:<addr>`
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Libraries {
    /// All libraries, `(file path -> (Lib name -> Address))`.
    pub libs: BTreeMap<PathBuf, BTreeMap<String, String>>,
}

// === impl Libraries ===

impl Libraries {
    /// Parses all libraries in the form of
    /// `<file>:<lib>:<addr>`
    ///
    /// # Examples
    ///
    /// ```
    /// use foundry_compilers_artifacts_solc::Libraries;
    ///
    /// let libs = Libraries::parse(&[
    ///     "src/DssSpell.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string(),
    /// ])?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(libs: &[String]) -> Result<Self, SolcError> {
        let mut libraries = BTreeMap::default();
        for lib in libs {
            let mut items = lib.split(':');
            let file = items.next().ok_or_else(|| {
                SolcError::msg(format!("failed to parse path to library file: {lib}"))
            })?;
            let lib = items
                .next()
                .ok_or_else(|| SolcError::msg(format!("failed to parse library name: {lib}")))?;
            let addr = items
                .next()
                .ok_or_else(|| SolcError::msg(format!("failed to parse library address: {lib}")))?;
            if items.next().is_some() {
                return Err(SolcError::msg(format!(
                    "failed to parse, too many arguments passed: {lib}"
                )));
            }
            libraries
                .entry(file.into())
                .or_insert_with(BTreeMap::default)
                .insert(lib.to_string(), addr.to_string());
        }
        Ok(Self { libs: libraries })
    }

    pub fn is_empty(&self) -> bool {
        self.libs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.libs.len()
    }

    /// Applies the given function to [Self] and returns the result.
    pub fn apply<F: FnOnce(Self) -> Self>(self, f: F) -> Self {
        f(self)
    }

    /// Strips the given prefix from all library file paths to make them relative to the given
    /// `base` argument
    pub fn with_stripped_file_prefixes(mut self, base: &Path) -> Self {
        self.libs = self
            .libs
            .into_iter()
            .map(|(f, l)| (f.strip_prefix(base).unwrap_or(&f).to_path_buf(), l))
            .collect();
        self
    }

    /// Converts all `\\` separators in _all_ paths to `/`
    pub fn slash_paths(&mut self) {
        #[cfg(windows)]
        {
            use path_slash::PathBufExt;

            self.libs = std::mem::take(&mut self.libs)
                .into_iter()
                .map(|(path, libs)| (PathBuf::from(path.to_slash_lossy().as_ref()), libs))
                .collect()
        }
    }
}

impl From<BTreeMap<PathBuf, BTreeMap<String, String>>> for Libraries {
    fn from(libs: BTreeMap<PathBuf, BTreeMap<String, String>>) -> Self {
        Self { libs }
    }
}

impl AsRef<BTreeMap<PathBuf, BTreeMap<String, String>>> for Libraries {
    fn as_ref(&self) -> &BTreeMap<PathBuf, BTreeMap<String, String>> {
        &self.libs
    }
}

impl AsMut<BTreeMap<PathBuf, BTreeMap<String, String>>> for Libraries {
    fn as_mut(&mut self) -> &mut BTreeMap<PathBuf, BTreeMap<String, String>> {
        &mut self.libs
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Optimizer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<usize>,
    /// Switch optimizer components on or off in detail.
    /// The "enabled" switch above provides two defaults which can be
    /// tweaked here. If "details" is given, "enabled" can be omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<OptimizerDetails>,
}

impl Optimizer {
    pub fn runs(&mut self, runs: usize) {
        self.runs = Some(runs);
    }

    pub fn disable(&mut self) {
        self.enabled.take();
    }

    pub fn enable(&mut self) {
        self.enabled = Some(true)
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self { enabled: Some(false), runs: Some(200), details: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerDetails {
    /// The peephole optimizer is always on if no details are given,
    /// use details to switch it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peephole: Option<bool>,
    /// The inliner is always on if no details are given,
    /// use details to switch it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inliner: Option<bool>,
    /// The unused jumpdest remover is always on if no details are given,
    /// use details to switch it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jumpdest_remover: Option<bool>,
    /// Sometimes re-orders literals in commutative operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_literals: Option<bool>,
    /// Removes duplicate code blocks
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduplicate: Option<bool>,
    /// Common subexpression elimination, this is the most complicated step but
    /// can also provide the largest gain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cse: Option<bool>,
    /// Optimize representation of literal numbers and strings in code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constant_optimizer: Option<bool>,
    /// The new Yul optimizer. Mostly operates on the code of ABI coder v2
    /// and inline assembly.
    /// It is activated together with the global optimizer setting
    /// and can be deactivated here.
    /// Before Solidity 0.6.0 it had to be activated through this switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yul: Option<bool>,
    /// Tuning options for the Yul optimizer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yul_details: Option<YulDetails>,
    /// Use unchecked arithmetic when incrementing the counter of for loops
    /// under certain circumstances. It is always on if no details are given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simple_counter_for_loop_unchecked_increment: Option<bool>,
}

// === impl OptimizerDetails ===

impl OptimizerDetails {
    /// Returns true if no settings are set.
    pub fn is_empty(&self) -> bool {
        self.peephole.is_none()
            && self.inliner.is_none()
            && self.jumpdest_remover.is_none()
            && self.order_literals.is_none()
            && self.deduplicate.is_none()
            && self.cse.is_none()
            && self.constant_optimizer.is_none()
            && self.yul.is_none()
            && self.yul_details.as_ref().map(|yul| yul.is_empty()).unwrap_or(true)
            && self.simple_counter_for_loop_unchecked_increment.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YulDetails {
    /// Improve allocation of stack slots for variables, can free up stack slots early.
    /// Activated by default if the Yul optimizer is activated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_allocation: Option<bool>,
    /// Select optimization steps to be applied.
    /// Optional, the optimizer will use the default sequence if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer_steps: Option<String>,
}

// === impl YulDetails ===

impl YulDetails {
    /// Returns true if no settings are set.
    pub fn is_empty(&self) -> bool {
        self.stack_allocation.is_none() && self.optimizer_steps.is_none()
    }
}

/// EVM versions.
///
/// Default is `Cancun`, since 0.8.25
///
/// Kept in sync with: <https://github.com/ethereum/solidity/blob/develop/liblangutil/EVMVersion.h>
// When adding new EVM versions (see a previous attempt at https://github.com/foundry-rs/compilers/pull/51):
// - add the version to the end of the enum
// - update the default variant to `m_version` default: https://github.com/ethereum/solidity/blob/develop/liblangutil/EVMVersion.h#L122
// - create a constant for the Solc version that introduced it in `../compile/mod.rs`
// - add the version to the top of `normalize_version` and wherever else the compiler complains
// - update `FromStr` impl
// - write a test case in `test_evm_version_normalization` at the bottom of this file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvmVersion {
    Homestead,
    TangerineWhistle,
    SpuriousDragon,
    Byzantium,
    Constantinople,
    Petersburg,
    Istanbul,
    Berlin,
    London,
    Paris,
    Shanghai,
    #[default]
    Cancun,
    Prague,
}

impl EvmVersion {
    /// Find the default EVM version for the given compiler version.
    pub fn default_version_solc(version: &Version) -> Option<Self> {
        // In most cases, Solc compilers use the highest EVM version available at the time.
        let default = Self::default().normalize_version_solc(version)?;

        // However, there are some exceptions where the default is lower than the highest available.
        match default {
            Self::Constantinople => {
                // Actually, Constantinople is never used as the default EVM version by Solidity
                // compilers.
                Some(Self::Byzantium)
            }
            Self::Cancun if *version == Version::new(0, 8, 24) => {
                // While Cancun is introduced at the time of releasing 0.8.24, it has not been
                // supported by the mainnet. So, the default EVM version of Solc 0.8.24 remains as
                // Shanghai.
                //
                // <https://soliditylang.org/blog/2024/01/26/solidity-0.8.24-release-announcement/>
                Some(Self::Shanghai)
            }
            Self::Prague if *version == Version::new(0, 8, 27) => {
                // Prague was not set as default EVM version in 0.8.27.
                Some(Self::Cancun)
            }
            _ => Some(default),
        }
    }

    /// Normalizes this EVM version by checking against the given Solc [`Version`].
    pub fn normalize_version_solc(self, version: &Version) -> Option<Self> {
        // The EVM version flag was only added in 0.4.21; we work our way backwards
        if *version >= BYZANTIUM_SOLC {
            // If the Solc version is the latest, it supports all EVM versions.
            // For all other cases, cap at the at-the-time highest possible fork.
            let normalized = if *version >= PRAGUE_SOLC {
                self
            } else if self >= Self::Cancun && *version >= CANCUN_SOLC {
                Self::Cancun
            } else if self >= Self::Shanghai && *version >= SHANGHAI_SOLC {
                Self::Shanghai
            } else if self >= Self::Paris && *version >= PARIS_SOLC {
                Self::Paris
            } else if self >= Self::London && *version >= LONDON_SOLC {
                Self::London
            } else if self >= Self::Berlin && *version >= BERLIN_SOLC {
                Self::Berlin
            } else if self >= Self::Istanbul && *version >= ISTANBUL_SOLC {
                Self::Istanbul
            } else if self >= Self::Petersburg && *version >= PETERSBURG_SOLC {
                Self::Petersburg
            } else if self >= Self::Constantinople && *version >= CONSTANTINOPLE_SOLC {
                Self::Constantinople
            } else if self >= Self::Byzantium {
                Self::Byzantium
            } else {
                self
            };
            Some(normalized)
        } else {
            None
        }
    }

    /// Returns the EVM version as a string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Homestead => "homestead",
            Self::TangerineWhistle => "tangerineWhistle",
            Self::SpuriousDragon => "spuriousDragon",
            Self::Byzantium => "byzantium",
            Self::Constantinople => "constantinople",
            Self::Petersburg => "petersburg",
            Self::Istanbul => "istanbul",
            Self::Berlin => "berlin",
            Self::London => "london",
            Self::Paris => "paris",
            Self::Shanghai => "shanghai",
            Self::Cancun => "cancun",
            Self::Prague => "prague",
        }
    }

    /// Has the `RETURNDATACOPY` and `RETURNDATASIZE` opcodes.
    pub fn supports_returndata(&self) -> bool {
        *self >= Self::Byzantium
    }

    pub fn has_static_call(&self) -> bool {
        *self >= Self::Byzantium
    }

    pub fn has_bitwise_shifting(&self) -> bool {
        *self >= Self::Constantinople
    }

    pub fn has_create2(&self) -> bool {
        *self >= Self::Constantinople
    }

    pub fn has_ext_code_hash(&self) -> bool {
        *self >= Self::Constantinople
    }

    pub fn has_chain_id(&self) -> bool {
        *self >= Self::Istanbul
    }

    pub fn has_self_balance(&self) -> bool {
        *self >= Self::Istanbul
    }

    pub fn has_base_fee(&self) -> bool {
        *self >= Self::London
    }

    pub fn has_prevrandao(&self) -> bool {
        *self >= Self::Paris
    }

    pub fn has_push0(&self) -> bool {
        *self >= Self::Shanghai
    }
}

impl fmt::Display for EvmVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EvmVersion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "homestead" => Ok(Self::Homestead),
            "tangerineWhistle" | "tangerinewhistle" => Ok(Self::TangerineWhistle),
            "spuriousDragon" | "spuriousdragon" => Ok(Self::SpuriousDragon),
            "byzantium" => Ok(Self::Byzantium),
            "constantinople" => Ok(Self::Constantinople),
            "petersburg" => Ok(Self::Petersburg),
            "istanbul" => Ok(Self::Istanbul),
            "berlin" => Ok(Self::Berlin),
            "london" => Ok(Self::London),
            "paris" => Ok(Self::Paris),
            "shanghai" => Ok(Self::Shanghai),
            "cancun" => Ok(Self::Cancun),
            "prague" => Ok(Self::Prague),
            s => Err(format!("Unknown evm version: {s}")),
        }
    }
}

/// Debugging settings for solc
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebuggingSettings {
    #[serde(
        default,
        with = "serde_helpers::display_from_str_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub revert_strings: Option<RevertStrings>,
    /// How much extra debug information to include in comments in the produced EVM assembly and
    /// Yul code.
    /// Available components are:
    // - `location`: Annotations of the form `@src <index>:<start>:<end>` indicating the location of
    //   the corresponding element in the original Solidity file, where:
    //     - `<index>` is the file index matching the `@use-src` annotation,
    //     - `<start>` is the index of the first byte at that location,
    //     - `<end>` is the index of the first byte after that location.
    // - `snippet`: A single-line code snippet from the location indicated by `@src`. The snippet is
    //   quoted and follows the corresponding `@src` annotation.
    // - `*`: Wildcard value that can be used to request everything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub debug_info: Vec<String>,
}

/// How to treat revert (and require) reason strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RevertStrings {
    /// "default" does not inject compiler-generated revert strings and keeps user-supplied ones.
    #[default]
    Default,
    /// "strip" removes all revert strings (if possible, i.e. if literals are used) keeping
    /// side-effects
    Strip,
    /// "debug" injects strings for compiler-generated internal reverts, implemented for ABI
    /// encoders V1 and V2 for now.
    Debug,
    /// "verboseDebug" even appends further information to user-supplied revert strings (not yet
    /// implemented)
    VerboseDebug,
}

impl fmt::Display for RevertStrings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            Self::Default => "default",
            Self::Strip => "strip",
            Self::Debug => "debug",
            Self::VerboseDebug => "verboseDebug",
        };
        write!(f, "{string}")
    }
}

impl FromStr for RevertStrings {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "default" => Ok(Self::Default),
            "strip" => Ok(Self::Strip),
            "debug" => Ok(Self::Debug),
            "verboseDebug" | "verbosedebug" => Ok(Self::VerboseDebug),
            s => Err(format!("Unknown revert string mode: {s}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsMetadata {
    /// Use only literal content and not URLs (false by default)
    #[serde(default, rename = "useLiteralContent", skip_serializing_if = "Option::is_none")]
    pub use_literal_content: Option<bool>,
    /// Use the given hash method for the metadata hash that is appended to the bytecode.
    /// The metadata hash can be removed from the bytecode via option "none".
    /// The other options are "ipfs" and "bzzr1".
    /// If the option is omitted, "ipfs" is used by default.
    #[serde(
        default,
        rename = "bytecodeHash",
        skip_serializing_if = "Option::is_none",
        with = "serde_helpers::display_from_str_opt"
    )]
    pub bytecode_hash: Option<BytecodeHash>,
    #[serde(default, rename = "appendCBOR", skip_serializing_if = "Option::is_none")]
    pub cbor_metadata: Option<bool>,
}

impl SettingsMetadata {
    pub fn new(hash: BytecodeHash, cbor: bool) -> Self {
        Self { use_literal_content: None, bytecode_hash: Some(hash), cbor_metadata: Some(cbor) }
    }
}

impl From<BytecodeHash> for SettingsMetadata {
    fn from(hash: BytecodeHash) -> Self {
        Self { use_literal_content: None, bytecode_hash: Some(hash), cbor_metadata: None }
    }
}

/// Determines the hash method for the metadata hash that is appended to the bytecode.
///
/// Solc's default is `Ipfs`, see <https://docs.soliditylang.org/en/latest/using-the-compiler.html#compiler-api>.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BytecodeHash {
    #[default]
    Ipfs,
    None,
    Bzzr1,
}

impl FromStr for BytecodeHash {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "ipfs" => Ok(Self::Ipfs),
            "bzzr1" => Ok(Self::Bzzr1),
            s => Err(format!("Unknown bytecode hash: {s}")),
        }
    }
}

impl fmt::Display for BytecodeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Ipfs => "ipfs",
            Self::None => "none",
            Self::Bzzr1 => "bzzr1",
        };
        f.write_str(s)
    }
}

/// Bindings for [`solc` contract metadata](https://docs.soliditylang.org/en/latest/metadata.html)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub compiler: Compiler,
    pub language: String,
    pub output: Output,
    pub settings: MetadataSettings,
    pub sources: MetadataSources,
    pub version: i64,
}

/// A helper type that ensures lossless (de)serialisation so we can preserve the exact String
/// metadata value that's being hashed by solc
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LosslessMetadata {
    /// The complete abi as json value
    pub raw_metadata: String,
    /// The deserialised metadata of `raw_metadata`
    pub metadata: Metadata,
}

// === impl LosslessMetadata ===

impl LosslessMetadata {
    /// Returns the whole string raw metadata as `serde_json::Value`
    pub fn raw_json(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::from_str(&self.raw_metadata)
    }
}

impl Serialize for LosslessMetadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.raw_metadata)
    }
}

impl<'de> Deserialize<'de> for LosslessMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LosslessMetadataVisitor;

        impl Visitor<'_> for LosslessMetadataVisitor {
            type Value = LosslessMetadata;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "metadata string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let metadata = serde_json::from_str(value).map_err(serde::de::Error::custom)?;
                let raw_metadata = value.to_string();
                Ok(LosslessMetadata { raw_metadata, metadata })
            }
        }
        deserializer.deserialize_str(LosslessMetadataVisitor)
    }
}

/// Compiler settings
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSettings {
    #[serde(default)]
    pub remappings: Vec<Remapping>,
    pub optimizer: Optimizer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SettingsMetadata>,
    /// Required for Solidity: File and name of the contract or library this metadata is created
    /// for.
    #[serde(default, rename = "compilationTarget")]
    pub compilation_target: BTreeMap<String, String>,
    // Introduced in 0.8.20
    #[serde(
        default,
        rename = "evmVersion",
        with = "serde_helpers::display_from_str_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub evm_version: Option<EvmVersion>,
    /// Metadata settings
    ///
    /// Note: this differs from `Libraries` and does not require another mapping for file name
    /// since metadata is per file
    #[serde(default)]
    pub libraries: BTreeMap<String, String>,
    /// Change compilation pipeline to go through the Yul intermediate representation. This is
    /// false by default.
    #[serde(rename = "viaIR", default, skip_serializing_if = "Option::is_none")]
    pub via_ir: Option<bool>,
}

/// Compilation source files/source units, keys are file names
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSources {
    #[serde(flatten)]
    pub inner: BTreeMap<String, MetadataSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSource {
    /// Required: keccak256 hash of the source file
    pub keccak256: String,
    /// Required (unless "content" is used, see below): Sorted URL(s)
    /// to the source file, protocol is more or less arbitrary, but a
    /// Swarm URL is recommended
    #[serde(default)]
    pub urls: Vec<String>,
    /// Required (unless "url" is used): literal contents of the source file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Optional: SPDX license identifier as given in the source file
    pub license: Option<String>,
}

/// Model checker settings for solc
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCheckerSettings {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub contracts: BTreeMap<String, Vec<String>>,
    #[serde(
        default,
        with = "serde_helpers::display_from_str_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub engine: Option<ModelCheckerEngine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<ModelCheckerTarget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invariants: Option<Vec<ModelCheckerInvariant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_unproved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub div_mod_with_slacks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solvers: Option<Vec<ModelCheckerSolver>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_unsupported: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_proved_safe: Option<bool>,
}

/// Which model checker engine to run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelCheckerEngine {
    #[default]
    Default,
    All,
    BMC,
    CHC,
}

impl fmt::Display for ModelCheckerEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            Self::Default => "none",
            Self::All => "all",
            Self::BMC => "bmc",
            Self::CHC => "chc",
        };
        write!(f, "{string}")
    }
}

impl FromStr for ModelCheckerEngine {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::Default),
            "all" => Ok(Self::All),
            "bmc" => Ok(Self::BMC),
            "chc" => Ok(Self::CHC),
            s => Err(format!("Unknown model checker engine: {s}")),
        }
    }
}

/// Which model checker targets to check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelCheckerTarget {
    Assert,
    Underflow,
    Overflow,
    DivByZero,
    ConstantCondition,
    PopEmptyArray,
    OutOfBounds,
    Balance,
}

impl fmt::Display for ModelCheckerTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            Self::Assert => "assert",
            Self::Underflow => "underflow",
            Self::Overflow => "overflow",
            Self::DivByZero => "divByZero",
            Self::ConstantCondition => "constantCondition",
            Self::PopEmptyArray => "popEmptyArray",
            Self::OutOfBounds => "outOfBounds",
            Self::Balance => "balance",
        };
        write!(f, "{string}")
    }
}

impl FromStr for ModelCheckerTarget {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "assert" => Ok(Self::Assert),
            "underflow" => Ok(Self::Underflow),
            "overflow" => Ok(Self::Overflow),
            "divByZero" => Ok(Self::DivByZero),
            "constantCondition" => Ok(Self::ConstantCondition),
            "popEmptyArray" => Ok(Self::PopEmptyArray),
            "outOfBounds" => Ok(Self::OutOfBounds),
            "balance" => Ok(Self::Balance),
            s => Err(format!("Unknown model checker target: {s}")),
        }
    }
}

/// Which model checker invariants to check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelCheckerInvariant {
    Contract,
    Reentrancy,
}

impl fmt::Display for ModelCheckerInvariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            Self::Contract => "contract",
            Self::Reentrancy => "reentrancy",
        };
        write!(f, "{string}")
    }
}

impl FromStr for ModelCheckerInvariant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contract" => Ok(Self::Contract),
            "reentrancy" => Ok(Self::Reentrancy),
            s => Err(format!("Unknown model checker invariant: {s}")),
        }
    }
}

/// Which model checker solvers to check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelCheckerSolver {
    Cvc4,
    Eld,
    Smtlib2,
    Z3,
}

impl fmt::Display for ModelCheckerSolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            Self::Cvc4 => "cvc4",
            Self::Eld => "eld",
            Self::Smtlib2 => "smtlib2",
            Self::Z3 => "z3",
        };
        write!(f, "{string}")
    }
}

impl FromStr for ModelCheckerSolver {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cvc4" => Ok(Self::Cvc4),
            "eld" => Ok(Self::Cvc4),
            "smtlib2" => Ok(Self::Smtlib2),
            "z3" => Ok(Self::Z3),
            s => Err(format!("Unknown model checker invariant: {s}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compiler {
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub abi: Vec<SolcAbi>,
    pub devdoc: Option<Doc>,
    pub userdoc: Option<Doc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolcAbi {
    #[serde(default)]
    pub inputs: Vec<Item>,
    #[serde(rename = "stateMutability", skip_serializing_if = "Option::is_none")]
    pub state_mutability: Option<String>,
    #[serde(rename = "type")]
    pub abi_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Item>,
    // required to satisfy solidity events
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymous: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    #[serde(rename = "internalType")]
    pub internal_type: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub put_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Item>,
    /// Indexed flag. for solidity events
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub methods: Option<DocLibraries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocLibraries {
    #[serde(flatten)]
    pub libs: BTreeMap<String, serde_json::Value>,
}

/// Output type `solc` produces
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Error>,
    #[serde(default)]
    pub sources: BTreeMap<PathBuf, SourceFile>,
    #[serde(default)]
    pub contracts: Contracts,
}

impl CompilerOutput {
    /// Whether the output contains a compiler error
    pub fn has_error(&self) -> bool {
        self.errors.iter().any(|err| err.severity.is_error())
    }

    /// Finds the _first_ contract with the given name
    pub fn find(&self, contract_name: &str) -> Option<CompactContractRef<'_>> {
        self.contracts_iter().find_map(|(name, contract)| {
            (name == contract_name).then(|| CompactContractRef::from(contract))
        })
    }

    /// Finds the first contract with the given name and removes it from the set
    pub fn remove(&mut self, contract_name: &str) -> Option<Contract> {
        self.contracts.values_mut().find_map(|c| c.remove(contract_name))
    }

    /// Iterate over all contracts and their names
    pub fn contracts_iter(&self) -> impl Iterator<Item = (&String, &Contract)> {
        self.contracts.values().flatten()
    }

    /// Iterate over all contracts and their names
    pub fn contracts_into_iter(self) -> impl Iterator<Item = (String, Contract)> {
        self.contracts.into_values().flatten()
    }

    /// Given the contract file's path and the contract's name, tries to return the contract's
    /// bytecode, runtime bytecode, and abi
    pub fn get(&self, path: &Path, contract: &str) -> Option<CompactContractRef<'_>> {
        self.contracts
            .get(path)
            .and_then(|contracts| contracts.get(contract))
            .map(CompactContractRef::from)
    }

    /// Returns the output's source files and contracts separately, wrapped in helper types that
    /// provide several helper methods
    pub fn split(self) -> (SourceFiles, OutputContracts) {
        (SourceFiles(self.sources), OutputContracts(self.contracts))
    }

    /// Retains only those files the given iterator yields
    ///
    /// In other words, removes all contracts for files not included in the iterator
    pub fn retain_files<'a, I>(&mut self, files: I)
    where
        I: IntoIterator<Item = &'a Path>,
    {
        // Note: use `to_lowercase` here because solc not necessarily emits the exact file name,
        // e.g. `src/utils/upgradeProxy.sol` is emitted as `src/utils/UpgradeProxy.sol`
        let files: HashSet<_> =
            files.into_iter().map(|s| s.to_string_lossy().to_lowercase()).collect();
        self.contracts.retain(|f, _| files.contains(&f.to_string_lossy().to_lowercase()));
        self.sources.retain(|f, _| files.contains(&f.to_string_lossy().to_lowercase()));
    }

    pub fn merge(&mut self, other: Self) {
        self.errors.extend(other.errors);
        self.contracts.extend(other.contracts);
        self.sources.extend(other.sources);
    }
}

/// A wrapper helper type for the `Contracts` type alias
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputContracts(pub Contracts);

impl OutputContracts {
    /// Returns an iterator over all contracts and their source names.
    pub fn into_contracts(self) -> impl Iterator<Item = (String, Contract)> {
        self.0.into_values().flatten()
    }

    /// Iterate over all contracts and their names
    pub fn contracts_iter(&self) -> impl Iterator<Item = (&String, &Contract)> {
        self.0.values().flatten()
    }

    /// Finds the _first_ contract with the given name
    pub fn find(&self, contract_name: &str) -> Option<CompactContractRef<'_>> {
        self.contracts_iter().find_map(|(name, contract)| {
            (name == contract_name).then(|| CompactContractRef::from(contract))
        })
    }

    /// Finds the first contract with the given name and removes it from the set
    pub fn remove(&mut self, contract_name: &str) -> Option<Contract> {
        self.0.values_mut().find_map(|c| c.remove(contract_name))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub methods: BTreeMap<String, UserDocNotice>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub events: BTreeMap<String, UserDocNotice>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub errors: BTreeMap<String, Vec<UserDocNotice>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserDocNotice {
    // NOTE: this a variant used for constructors on older solc versions
    Constructor(String),
    Notice { notice: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, rename = "custom:experimental", skip_serializing_if = "Option::is_none")]
    pub custom_experimental: Option<String>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub methods: BTreeMap<String, MethodDoc>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub events: BTreeMap<String, EventDoc>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub errors: BTreeMap<String, Vec<ErrorDoc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub returns: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evm {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_assembly: Option<serde_json::Value>,
    pub bytecode: Option<Bytecode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_bytecode: Option<DeployedBytecode>,
    /// The list of function hashes
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub method_identifiers: BTreeMap<String, String>,
    /// Function gas estimates
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_estimates: Option<GasEstimates>,
}

impl Evm {
    /// Crate internal helper do transform the underlying bytecode artifacts into a more convenient
    /// structure
    pub(crate) fn into_compact(self) -> CompactEvm {
        let Self {
            assembly,
            legacy_assembly,
            bytecode,
            deployed_bytecode,
            method_identifiers,
            gas_estimates,
        } = self;

        let (bytecode, deployed_bytecode) = match (bytecode, deployed_bytecode) {
            (Some(bcode), Some(dbcode)) => (Some(bcode.into()), Some(dbcode.into())),
            (None, Some(dbcode)) => (None, Some(dbcode.into())),
            (Some(bcode), None) => (Some(bcode.into()), None),
            (None, None) => (None, None),
        };

        CompactEvm {
            assembly,
            legacy_assembly,
            bytecode,
            deployed_bytecode,
            method_identifiers,
            gas_estimates,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompactEvm {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_assembly: Option<serde_json::Value>,
    pub bytecode: Option<CompactBytecode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_bytecode: Option<CompactDeployedBytecode>,
    /// The list of function hashes
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub method_identifiers: BTreeMap<String, String>,
    /// Function gas estimates
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_estimates: Option<GasEstimates>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDebugData {
    pub entry_point: Option<u32>,
    pub id: Option<u32>,
    pub parameter_slots: Option<u32>,
    pub return_slots: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedSource {
    pub ast: serde_json::Value,
    pub contents: String,
    pub id: u32,
    pub language: String,
    pub name: String,
}

/// Byte offsets into the bytecode.
/// Linking replaces the 20 bytes located there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offsets {
    pub start: u32,
    pub length: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasEstimates {
    pub creation: Creation,
    #[serde(default)]
    pub external: BTreeMap<String, String>,
    #[serde(default)]
    pub internal: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creation {
    pub code_deposit_cost: String,
    pub execution_cost: String,
    pub total_cost: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ewasm {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wast: Option<String>,
    pub wasm: String,
}

/// Represents the `storage-layout` section of the `CompilerOutput` if selected.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageLayout {
    pub storage: Vec<Storage>,
    #[serde(default, deserialize_with = "serde_helpers::default_for_null")]
    pub types: BTreeMap<String, StorageType>,
}

impl StorageLayout {
    fn is_empty(&self) -> bool {
        self.storage.is_empty() && self.types.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Storage {
    #[serde(rename = "astId")]
    pub ast_id: u64,
    pub contract: String,
    pub label: String,
    pub offset: i64,
    pub slot: String,
    #[serde(rename = "type")]
    pub storage_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageType {
    pub encoding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub label: String,
    #[serde(rename = "numberOfBytes")]
    pub number_of_bytes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// additional fields
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: u32,
    #[serde(default, with = "serde_helpers::empty_json_object_opt")]
    pub ast: Option<Ast>,
}

impl SourceFile {
    /// Returns `true` if the source file contains at least 1 `ContractDefinition` such as
    /// `contract`, `abstract contract`, `interface` or `library`.
    pub fn contains_contract_definition(&self) -> bool {
        self.ast.as_ref().is_some_and(|ast| {
            ast.nodes.iter().any(|node| matches!(node.node_type, NodeType::ContractDefinition))
        })
    }
}

/// A wrapper type for a list of source files: `path -> SourceFile`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFiles(pub BTreeMap<PathBuf, SourceFile>);

impl SourceFiles {
    /// Returns an iterator over the source files' IDs and path.
    pub fn into_ids(self) -> impl Iterator<Item = (u32, PathBuf)> {
        self.0.into_iter().map(|(k, v)| (v.id, k))
    }

    /// Returns an iterator over the source files' paths and IDs.
    pub fn into_paths(self) -> impl Iterator<Item = (PathBuf, u32)> {
        self.0.into_iter().map(|(k, v)| (k, v.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use similar_asserts::assert_eq;
    use std::fs;

    #[test]
    fn can_link_bytecode() {
        // test cases taken from <https://github.com/ethereum/solc-js/blob/master/test/linker.js>

        #[derive(Serialize, Deserialize)]
        struct Mockject {
            object: BytecodeObject,
        }
        fn parse_bytecode(bytecode: &str) -> BytecodeObject {
            let object: Mockject =
                serde_json::from_value(serde_json::json!({ "object": bytecode })).unwrap();
            object.object
        }

        let bytecode =  "6060604052341561000f57600080fd5b60f48061001d6000396000f300606060405260043610603e5763ffffffff7c010000000000000000000000000000000000000000000000000000000060003504166326121ff081146043575b600080fd5b3415604d57600080fd5b60536055565b005b73__lib2.sol:L____________________________6326121ff06040518163ffffffff167c010000000000000000000000000000000000000000000000000000000002815260040160006040518083038186803b151560b357600080fd5b6102c65a03f4151560c357600080fd5b5050505600a165627a7a723058207979b30bd4a07c77b02774a511f2a1dd04d7e5d65b5c2735b5fc96ad61d43ae40029";

        let mut object = parse_bytecode(bytecode);
        assert!(object.is_unlinked());
        assert!(object.contains_placeholder("lib2.sol", "L"));
        assert!(object.contains_fully_qualified_placeholder("lib2.sol:L"));
        assert!(object.link("lib2.sol", "L", Address::random()).resolve().is_some());
        assert!(!object.is_unlinked());

        let mut code = Bytecode {
            function_debug_data: Default::default(),
            object: parse_bytecode(bytecode),
            opcodes: None,
            source_map: None,
            generated_sources: vec![],
            link_references: BTreeMap::from([(
                "lib2.sol".to_string(),
                BTreeMap::from([("L".to_string(), vec![])]),
            )]),
        };

        assert!(!code.link("lib2.sol", "Y", Address::random()));
        assert!(code.link("lib2.sol", "L", Address::random()));
        assert!(code.link("lib2.sol", "L", Address::random()));

        let hashed_placeholder = "6060604052341561000f57600080fd5b60f48061001d6000396000f300606060405260043610603e5763ffffffff7c010000000000000000000000000000000000000000000000000000000060003504166326121ff081146043575b600080fd5b3415604d57600080fd5b60536055565b005b73__$cb901161e812ceb78cfe30ca65050c4337$__6326121ff06040518163ffffffff167c010000000000000000000000000000000000000000000000000000000002815260040160006040518083038186803b151560b357600080fd5b6102c65a03f4151560c357600080fd5b5050505600a165627a7a723058207979b30bd4a07c77b02774a511f2a1dd04d7e5d65b5c2735b5fc96ad61d43ae40029";
        let mut object = parse_bytecode(hashed_placeholder);
        assert!(object.is_unlinked());
        assert!(object.contains_placeholder("lib2.sol", "L"));
        assert!(object.contains_fully_qualified_placeholder("lib2.sol:L"));
        assert!(object.link("lib2.sol", "L", Address::default()).resolve().is_some());
        assert!(!object.is_unlinked());
    }

    #[test]
    fn can_parse_compiler_output() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test-data/out");

        for path in fs::read_dir(dir).unwrap() {
            let path = path.unwrap().path();
            let compiler_output = fs::read_to_string(&path).unwrap();
            serde_json::from_str::<CompilerOutput>(&compiler_output).unwrap_or_else(|err| {
                panic!("Failed to read compiler output of {} {}", path.display(), err)
            });
        }
    }

    #[test]
    fn can_parse_compiler_input() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test-data/in");

        for path in fs::read_dir(dir).unwrap() {
            let path = path.unwrap().path();
            let compiler_input = fs::read_to_string(&path).unwrap();
            serde_json::from_str::<SolcInput>(&compiler_input).unwrap_or_else(|err| {
                panic!("Failed to read compiler input of {} {}", path.display(), err)
            });
        }
    }

    #[test]
    fn can_parse_standard_json_compiler_input() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test-data/in");

        for path in fs::read_dir(dir).unwrap() {
            let path = path.unwrap().path();
            let compiler_input = fs::read_to_string(&path).unwrap();
            let val = serde_json::from_str::<StandardJsonCompilerInput>(&compiler_input)
                .unwrap_or_else(|err| {
                    panic!("Failed to read compiler output of {} {}", path.display(), err)
                });

            let pretty = serde_json::to_string_pretty(&val).unwrap();
            serde_json::from_str::<SolcInput>(&pretty).unwrap_or_else(|err| {
                panic!("Failed to read converted compiler input of {} {}", path.display(), err)
            });
        }
    }

    #[test]
    fn test_evm_version_default() {
        for &(solc_version, expected) in &[
            // Everything before 0.4.21 should always return None
            ("0.4.20", None),
            // Byzantium clipping
            ("0.4.21", Some(EvmVersion::Byzantium)),
            // Constantinople bug fix
            ("0.4.22", Some(EvmVersion::Byzantium)),
            // Petersburg
            ("0.5.5", Some(EvmVersion::Petersburg)),
            // Istanbul
            ("0.5.14", Some(EvmVersion::Istanbul)),
            // Berlin
            ("0.8.5", Some(EvmVersion::Berlin)),
            // London
            ("0.8.7", Some(EvmVersion::London)),
            // Paris
            ("0.8.18", Some(EvmVersion::Paris)),
            // Shanghai
            ("0.8.20", Some(EvmVersion::Shanghai)),
            // Cancun
            ("0.8.24", Some(EvmVersion::Shanghai)),
            ("0.8.25", Some(EvmVersion::Cancun)),
        ] {
            let version = Version::from_str(solc_version).unwrap();
            assert_eq!(
                EvmVersion::default_version_solc(&version),
                expected,
                "({version}, {expected:?})"
            )
        }
    }

    #[test]
    fn test_evm_version_normalization() {
        for &(solc_version, evm_version, expected) in &[
            // Everything before 0.4.21 should always return None
            ("0.4.20", EvmVersion::Homestead, None),
            // Byzantium clipping
            ("0.4.21", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.4.21", EvmVersion::Constantinople, Some(EvmVersion::Byzantium)),
            ("0.4.21", EvmVersion::London, Some(EvmVersion::Byzantium)),
            // Constantinople bug fix
            ("0.4.22", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.4.22", EvmVersion::Constantinople, Some(EvmVersion::Constantinople)),
            ("0.4.22", EvmVersion::London, Some(EvmVersion::Constantinople)),
            // Petersburg
            ("0.5.5", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.5.5", EvmVersion::Petersburg, Some(EvmVersion::Petersburg)),
            ("0.5.5", EvmVersion::London, Some(EvmVersion::Petersburg)),
            // Istanbul
            ("0.5.14", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.5.14", EvmVersion::Istanbul, Some(EvmVersion::Istanbul)),
            ("0.5.14", EvmVersion::London, Some(EvmVersion::Istanbul)),
            // Berlin
            ("0.8.5", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.8.5", EvmVersion::Berlin, Some(EvmVersion::Berlin)),
            ("0.8.5", EvmVersion::London, Some(EvmVersion::Berlin)),
            // London
            ("0.8.7", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.8.7", EvmVersion::London, Some(EvmVersion::London)),
            ("0.8.7", EvmVersion::Paris, Some(EvmVersion::London)),
            // Paris
            ("0.8.18", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.8.18", EvmVersion::Paris, Some(EvmVersion::Paris)),
            ("0.8.18", EvmVersion::Shanghai, Some(EvmVersion::Paris)),
            // Shanghai
            ("0.8.20", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.8.20", EvmVersion::Paris, Some(EvmVersion::Paris)),
            ("0.8.20", EvmVersion::Shanghai, Some(EvmVersion::Shanghai)),
            ("0.8.20", EvmVersion::Cancun, Some(EvmVersion::Shanghai)),
            // Cancun
            ("0.8.24", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.8.24", EvmVersion::Shanghai, Some(EvmVersion::Shanghai)),
            ("0.8.24", EvmVersion::Cancun, Some(EvmVersion::Cancun)),
            // Prague
            ("0.8.26", EvmVersion::Homestead, Some(EvmVersion::Homestead)),
            ("0.8.26", EvmVersion::Shanghai, Some(EvmVersion::Shanghai)),
            ("0.8.26", EvmVersion::Cancun, Some(EvmVersion::Cancun)),
            ("0.8.26", EvmVersion::Prague, Some(EvmVersion::Cancun)),
            ("0.8.27", EvmVersion::Prague, Some(EvmVersion::Prague)),
        ] {
            let version = Version::from_str(solc_version).unwrap();
            assert_eq!(
                evm_version.normalize_version_solc(&version),
                expected,
                "({version}, {evm_version:?})"
            )
        }
    }

    #[test]
    fn can_sanitize_byte_code_hash() {
        let settings = Settings { metadata: Some(BytecodeHash::Ipfs.into()), ..Default::default() };

        let input =
            SolcInput { language: SolcLanguage::Solidity, sources: Default::default(), settings };

        let i = input.clone().sanitized(&Version::new(0, 6, 0));
        assert_eq!(i.settings.metadata.unwrap().bytecode_hash, Some(BytecodeHash::Ipfs));

        let i = input.sanitized(&Version::new(0, 5, 17));
        assert!(i.settings.metadata.unwrap().bytecode_hash.is_none());
    }

    #[test]
    fn can_sanitize_cbor_metadata() {
        let settings = Settings {
            metadata: Some(SettingsMetadata::new(BytecodeHash::Ipfs, true)),
            ..Default::default()
        };

        let input =
            SolcInput { language: SolcLanguage::Solidity, sources: Default::default(), settings };

        let i = input.clone().sanitized(&Version::new(0, 8, 18));
        assert_eq!(i.settings.metadata.unwrap().cbor_metadata, Some(true));

        let i = input.sanitized(&Version::new(0, 8, 0));
        assert!(i.settings.metadata.unwrap().cbor_metadata.is_none());
    }

    #[test]
    fn can_parse_libraries() {
        let libraries = ["./src/lib/LibraryContract.sol:Library:0xaddress".to_string()];

        let libs = Libraries::parse(&libraries[..]).unwrap().libs;

        assert_eq!(
            libs,
            BTreeMap::from([(
                PathBuf::from("./src/lib/LibraryContract.sol"),
                BTreeMap::from([("Library".to_string(), "0xaddress".to_string())])
            )])
        );
    }

    #[test]
    fn can_strip_libraries_path_prefixes() {
        let libraries= [
            "/global/root/src/FileInSrc.sol:Chainlink:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string(),
            "src/deep/DeepFileInSrc.sol:ChainlinkTWAP:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string(),
            "/global/GlobalFile.sol:Math:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string(),
            "/global/root/test/ChainlinkTWAP.t.sol:ChainlinkTWAP:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string(),
            "test/SizeAuctionDiscount.sol:Math:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string(),
        ];

        let libs = Libraries::parse(&libraries[..])
            .unwrap()
            .with_stripped_file_prefixes("/global/root".as_ref())
            .libs;

        assert_eq!(
            libs,
            BTreeMap::from([
                (
                    PathBuf::from("/global/GlobalFile.sol"),
                    BTreeMap::from([(
                        "Math".to_string(),
                        "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                    )])
                ),
                (
                    PathBuf::from("src/FileInSrc.sol"),
                    BTreeMap::from([(
                        "Chainlink".to_string(),
                        "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                    )])
                ),
                (
                    PathBuf::from("src/deep/DeepFileInSrc.sol"),
                    BTreeMap::from([(
                        "ChainlinkTWAP".to_string(),
                        "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                    )])
                ),
                (
                    PathBuf::from("test/SizeAuctionDiscount.sol"),
                    BTreeMap::from([(
                        "Math".to_string(),
                        "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                    )])
                ),
                (
                    PathBuf::from("test/ChainlinkTWAP.t.sol"),
                    BTreeMap::from([(
                        "ChainlinkTWAP".to_string(),
                        "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                    )])
                ),
            ])
        );
    }

    #[test]
    fn can_parse_many_libraries() {
        let libraries= [
            "./src/SizeAuctionDiscount.sol:Chainlink:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string(),
            "./src/SizeAuction.sol:ChainlinkTWAP:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string(),
            "./src/SizeAuction.sol:Math:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string(),
            "./src/test/ChainlinkTWAP.t.sol:ChainlinkTWAP:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string(),
            "./src/SizeAuctionDiscount.sol:Math:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string(),
        ];

        let libs = Libraries::parse(&libraries[..]).unwrap().libs;

        assert_eq!(
            libs,
            BTreeMap::from([
                (
                    PathBuf::from("./src/SizeAuctionDiscount.sol"),
                    BTreeMap::from([
                        (
                            "Chainlink".to_string(),
                            "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                        ),
                        (
                            "Math".to_string(),
                            "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                        )
                    ])
                ),
                (
                    PathBuf::from("./src/SizeAuction.sol"),
                    BTreeMap::from([
                        (
                            "ChainlinkTWAP".to_string(),
                            "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                        ),
                        (
                            "Math".to_string(),
                            "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                        )
                    ])
                ),
                (
                    PathBuf::from("./src/test/ChainlinkTWAP.t.sol"),
                    BTreeMap::from([(
                        "ChainlinkTWAP".to_string(),
                        "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                    )])
                ),
            ])
        );
    }

    #[test]
    fn test_lossless_metadata() {
        #[derive(Debug, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Contract {
            #[serde(
                default,
                skip_serializing_if = "Option::is_none",
                with = "serde_helpers::json_string_opt"
            )]
            pub metadata: Option<LosslessMetadata>,
        }

        let s = r#"{"metadata":"{\"compiler\":{\"version\":\"0.4.18+commit.9cf6e910\"},\"language\":\"Solidity\",\"output\":{\"abi\":[{\"constant\":true,\"inputs\":[],\"name\":\"owner\",\"outputs\":[{\"name\":\"\",\"type\":\"address\"}],\"payable\":false,\"stateMutability\":\"view\",\"type\":\"function\"},{\"constant\":false,\"inputs\":[{\"name\":\"newOwner\",\"type\":\"address\"}],\"name\":\"transferOwnership\",\"outputs\":[],\"payable\":false,\"stateMutability\":\"nonpayable\",\"type\":\"function\"},{\"inputs\":[],\"payable\":false,\"stateMutability\":\"nonpayable\",\"type\":\"constructor\"}],\"devdoc\":{\"methods\":{\"transferOwnership(address)\":{\"details\":\"Allows the current owner to transfer control of the contract to a newOwner.\",\"params\":{\"newOwner\":\"The address to transfer ownership to.\"}}},\"title\":\"Ownable\"},\"userdoc\":{\"methods\":{}}},\"settings\":{\"compilationTarget\":{\"src/Contract.sol\":\"Ownable\"},\"libraries\":{},\"optimizer\":{\"enabled\":true,\"runs\":1000000},\"remappings\":[\":src/=src/\"]},\"sources\":{\"src/Contract.sol\":{\"keccak256\":\"0x3e0d611f53491f313ae035797ed7ecfd1dfd8db8fef8f82737e6f0cd86d71de7\",\"urls\":[\"bzzr://9c33025fa9d1b8389e4c7c9534a1d70fad91c6c2ad70eb5e4b7dc3a701a5f892\"]}},\"version\":1}"}"#;

        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let c: Contract = serde_json::from_value(value).unwrap();
        assert_eq!(c.metadata.as_ref().unwrap().raw_metadata, "{\"compiler\":{\"version\":\"0.4.18+commit.9cf6e910\"},\"language\":\"Solidity\",\"output\":{\"abi\":[{\"constant\":true,\"inputs\":[],\"name\":\"owner\",\"outputs\":[{\"name\":\"\",\"type\":\"address\"}],\"payable\":false,\"stateMutability\":\"view\",\"type\":\"function\"},{\"constant\":false,\"inputs\":[{\"name\":\"newOwner\",\"type\":\"address\"}],\"name\":\"transferOwnership\",\"outputs\":[],\"payable\":false,\"stateMutability\":\"nonpayable\",\"type\":\"function\"},{\"inputs\":[],\"payable\":false,\"stateMutability\":\"nonpayable\",\"type\":\"constructor\"}],\"devdoc\":{\"methods\":{\"transferOwnership(address)\":{\"details\":\"Allows the current owner to transfer control of the contract to a newOwner.\",\"params\":{\"newOwner\":\"The address to transfer ownership to.\"}}},\"title\":\"Ownable\"},\"userdoc\":{\"methods\":{}}},\"settings\":{\"compilationTarget\":{\"src/Contract.sol\":\"Ownable\"},\"libraries\":{},\"optimizer\":{\"enabled\":true,\"runs\":1000000},\"remappings\":[\":src/=src/\"]},\"sources\":{\"src/Contract.sol\":{\"keccak256\":\"0x3e0d611f53491f313ae035797ed7ecfd1dfd8db8fef8f82737e6f0cd86d71de7\",\"urls\":[\"bzzr://9c33025fa9d1b8389e4c7c9534a1d70fad91c6c2ad70eb5e4b7dc3a701a5f892\"]}},\"version\":1}");

        let value = serde_json::to_string(&c).unwrap();
        assert_eq!(s, value);
    }

    #[test]
    fn test_lossless_storage_layout() {
        let input = include_str!("../../../../test-data/foundryissue2462.json").trim();
        let layout: StorageLayout = serde_json::from_str(input).unwrap();
        assert_eq!(input, &serde_json::to_string(&layout).unwrap());
    }

    // <https://github.com/foundry-rs/foundry/issues/3012>
    #[test]
    fn can_parse_compiler_output_spells_0_6_12() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test-data/0.6.12-with-libs.json");
        let content = fs::read_to_string(path).unwrap();
        let _output: CompilerOutput = serde_json::from_str(&content).unwrap();
    }

    // <https://github.com/foundry-rs/foundry/issues/9322>
    #[test]
    fn can_sanitize_optimizer_inliner() {
        let settings = Settings::default().with_via_ir_minimum_optimization();

        let input =
            SolcInput { language: SolcLanguage::Solidity, sources: Default::default(), settings };

        let i = input.clone().sanitized(&Version::new(0, 8, 4));
        assert!(i.settings.optimizer.details.unwrap().inliner.is_none());

        let i = input.sanitized(&Version::new(0, 8, 5));
        assert_eq!(i.settings.optimizer.details.unwrap().inliner, Some(false));
    }
}
