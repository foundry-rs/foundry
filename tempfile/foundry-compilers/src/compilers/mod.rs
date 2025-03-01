use crate::ProjectPathsConfig;
use alloy_json_abi::JsonAbi;
use core::fmt;
use foundry_compilers_artifacts::{
    error::SourceLocation,
    output_selection::OutputSelection,
    remappings::Remapping,
    sources::{Source, Sources},
    BytecodeObject, CompactContractRef, Contract, FileToContractsMap, Severity, SourceFile,
};
use foundry_compilers_core::error::Result;
use semver::{Version, VersionReq};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt::{Debug, Display},
    hash::Hash,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub mod multi;
pub mod solc;
pub mod vyper;
pub use vyper::*;

mod restrictions;
pub use restrictions::{CompilerSettingsRestrictions, RestrictionsWithVersion};

/// A compiler version is either installed (available locally) or can be downloaded, from the remote
/// endpoint
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompilerVersion {
    Installed(Version),
    Remote(Version),
}

impl CompilerVersion {
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed(_))
    }
}

impl AsRef<Version> for CompilerVersion {
    fn as_ref(&self) -> &Version {
        match self {
            Self::Installed(v) | Self::Remote(v) => v,
        }
    }
}

impl From<CompilerVersion> for Version {
    fn from(s: CompilerVersion) -> Self {
        match s {
            CompilerVersion::Installed(v) | CompilerVersion::Remote(v) => v,
        }
    }
}

impl fmt::Display for CompilerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

/// Compilation settings including evm_version, output_selection, etc.
pub trait CompilerSettings:
    Default + Serialize + DeserializeOwned + Clone + Debug + Send + Sync + 'static
{
    /// We allow configuring settings restrictions which might optionally contain specific
    /// requiremets for compiler configuration. e.g. min/max evm_version, optimizer runs
    type Restrictions: CompilerSettingsRestrictions;

    /// Executes given fn with mutable reference to configured [OutputSelection].
    fn update_output_selection(&mut self, f: impl FnOnce(&mut OutputSelection) + Copy);

    /// Returns true if artifacts compiled with given `other` config are compatible with this
    /// config and if compilation can be skipped.
    ///
    /// Ensures that all settings fields are equal except for `output_selection` which is required
    /// to be a subset of `cached.output_selection`.
    fn can_use_cached(&self, other: &Self) -> bool;

    /// Method which might be invoked to add remappings to the input.
    fn with_remappings(self, _remappings: &[Remapping]) -> Self {
        self
    }

    /// Builder method to set the base path for the compiler. Primarily used by solc implementation
    /// to se --base-path.
    fn with_base_path(self, _base_path: &Path) -> Self {
        self
    }

    /// Builder method to set the allowed paths for the compiler. Primarily used by solc
    /// implementation to set --allow-paths.
    fn with_allow_paths(self, _allowed_paths: &BTreeSet<PathBuf>) -> Self {
        self
    }

    /// Builder method to set the include paths for the compiler. Primarily used by solc
    /// implementation to set --include-paths.
    fn with_include_paths(self, _include_paths: &BTreeSet<PathBuf>) -> Self {
        self
    }

    /// Returns whether current settings satisfy given restrictions.
    fn satisfies_restrictions(&self, restrictions: &Self::Restrictions) -> bool;
}

/// Input of a compiler, including sources and settings used for their compilation.
pub trait CompilerInput: Serialize + Send + Sync + Sized + Debug {
    type Settings: CompilerSettings;
    type Language: Language;

    /// Constructs one or multiple inputs from given sources set. Might return multiple inputs in
    /// cases when sources need to be divided into sets per language (Yul + Solidity for example).
    fn build(
        sources: Sources,
        settings: Self::Settings,
        language: Self::Language,
        version: Version,
    ) -> Self;

    /// Returns language of the sources included into this input.
    fn language(&self) -> Self::Language;

    /// Returns compiler version for which this input is intended.
    fn version(&self) -> &Version;

    fn sources(&self) -> impl Iterator<Item = (&Path, &Source)>;

    /// Returns compiler name used by reporters to display output during compilation.
    fn compiler_name(&self) -> Cow<'static, str>;

    /// Strips given prefix from all paths.
    fn strip_prefix(&mut self, base: &Path);
}

/// Parser of the source files which is used to identify imports and version requirements of the
/// given source.
///
/// Used by path resolver to resolve imports or determine compiler versions needed to compiler given
/// sources.
pub trait ParsedSource: Debug + Sized + Send + Clone {
    type Language: Language;

    /// Parses the content of the source file.
    fn parse(content: &str, file: &Path) -> Result<Self>;

    /// Returns the version requirement of the source.
    fn version_req(&self) -> Option<&VersionReq>;

    /// Returns a list of contract names defined in the source.
    fn contract_names(&self) -> &[String];

    /// Returns the language of the source.
    fn language(&self) -> Self::Language;

    /// Invoked during import resolution. Should resolve imports for the given source, and populate
    /// include_paths for compilers which support this config.
    fn resolve_imports<C>(
        &self,
        paths: &ProjectPathsConfig<C>,
        include_paths: &mut BTreeSet<PathBuf>,
    ) -> Result<Vec<PathBuf>>;

    /// Used to configure [OutputSelection] for sparse builds. In certain cases, we might want to
    /// include some of the file dependencies into the compiler output even if we might not be
    /// directly interested in them.
    ///
    /// Example of such case is when we are compiling Solidity file containing link references and
    /// need them to be included in the output to deploy needed libraries.
    ///
    /// Receives iterator over imports of the current source.
    ///
    /// Returns iterator over paths to the files that should be compiled with full output selection.
    fn compilation_dependencies<'a>(
        &self,
        _imported_nodes: impl Iterator<Item = (&'a Path, &'a Self)>,
    ) -> impl Iterator<Item = &'a Path>
    where
        Self: 'a,
    {
        vec![].into_iter()
    }
}

/// Error returned by compiler. Might also represent a warning or informational message.
pub trait CompilationError:
    Serialize + Send + Sync + Display + Debug + Clone + PartialEq + Eq + 'static
{
    fn is_warning(&self) -> bool;
    fn is_error(&self) -> bool;
    fn source_location(&self) -> Option<SourceLocation>;
    fn severity(&self) -> Severity;
    fn error_code(&self) -> Option<u64>;
}

/// Output of the compiler, including contracts, sources, errors and metadata. might be
/// extended to be more generic in the future.
#[derive(Debug, Serialize, Deserialize)]
pub struct CompilerOutput<E, C> {
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<E>,
    #[serde(default = "BTreeMap::new")]
    pub contracts: FileToContractsMap<C>,
    #[serde(default)]
    pub sources: BTreeMap<PathBuf, SourceFile>,
    #[serde(default, skip_serializing_if = "::std::collections::BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl<E, C> CompilerOutput<E, C> {
    /// Retains only those files the given iterator yields
    ///
    /// In other words, removes all contracts for files not included in the iterator
    pub fn retain_files<F, I>(&mut self, files: I)
    where
        F: AsRef<Path>,
        I: IntoIterator<Item = F>,
    {
        // Note: use `to_lowercase` here because solc not necessarily emits the exact file name,
        // e.g. `src/utils/upgradeProxy.sol` is emitted as `src/utils/UpgradeProxy.sol`
        let files: HashSet<_> =
            files.into_iter().map(|s| s.as_ref().to_string_lossy().to_lowercase()).collect();
        self.contracts.retain(|f, _| files.contains(&f.to_string_lossy().to_lowercase()));
        self.sources.retain(|f, _| files.contains(&f.to_string_lossy().to_lowercase()));
    }

    pub fn merge(&mut self, other: Self) {
        self.errors.extend(other.errors);
        self.contracts.extend(other.contracts);
        self.sources.extend(other.sources);
    }

    pub fn join_all(&mut self, root: &Path) {
        self.contracts = std::mem::take(&mut self.contracts)
            .into_iter()
            .map(|(path, contracts)| (root.join(path), contracts))
            .collect();
        self.sources = std::mem::take(&mut self.sources)
            .into_iter()
            .map(|(path, source)| (root.join(path), source))
            .collect();
    }

    pub fn map_err<F, O: FnMut(E) -> F>(self, op: O) -> CompilerOutput<F, C> {
        CompilerOutput {
            errors: self.errors.into_iter().map(op).collect(),
            contracts: self.contracts,
            sources: self.sources,
            metadata: self.metadata,
        }
    }
}

impl<E, C> Default for CompilerOutput<E, C> {
    fn default() -> Self {
        Self {
            errors: Vec::new(),
            contracts: BTreeMap::new(),
            sources: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }
}

/// Keeps a set of languages recognized by the compiler.
pub trait Language:
    Hash + Eq + Copy + Clone + Debug + Display + Send + Sync + Serialize + DeserializeOwned + 'static
{
    /// Extensions of source files recognized by the language set.
    const FILE_EXTENSIONS: &'static [&'static str];
}

/// Represents a compiled contract
pub trait CompilerContract: Serialize + Send + Sync + Debug + Clone + Eq + Sized {
    /// Reference to contract ABI
    fn abi_ref(&self) -> Option<&JsonAbi>;

    //// Reference to contract bytecode
    fn bin_ref(&self) -> Option<&BytecodeObject>;

    //// Reference to contract runtime bytecode
    fn bin_runtime_ref(&self) -> Option<&BytecodeObject>;

    fn as_compact_contract_ref(&self) -> CompactContractRef<'_> {
        CompactContractRef {
            abi: self.abi_ref(),
            bin: self.bin_ref(),
            bin_runtime: self.bin_runtime_ref(),
        }
    }
}

impl CompilerContract for Contract {
    fn abi_ref(&self) -> Option<&JsonAbi> {
        self.abi.as_ref()
    }
    fn bin_ref(&self) -> Option<&BytecodeObject> {
        if let Some(ref evm) = self.evm {
            evm.bytecode.as_ref().map(|c| &c.object)
        } else {
            None
        }
    }
    fn bin_runtime_ref(&self) -> Option<&BytecodeObject> {
        if let Some(ref evm) = self.evm {
            evm.deployed_bytecode
                .as_ref()
                .and_then(|deployed| deployed.bytecode.as_ref().map(|evm| &evm.object))
        } else {
            None
        }
    }
}

/// The main compiler abstraction trait.
///
/// Currently mostly represents a wrapper around compiler binary aware of the version and able to
/// compile given input into [`CompilerOutput`] including artifacts and errors.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait Compiler: Send + Sync + Clone {
    /// Input type for the compiler. Contains settings and sources to be compiled.
    type Input: CompilerInput<Settings = Self::Settings, Language = Self::Language>;
    /// Error type returned by the compiler.
    type CompilationError: CompilationError;
    /// Output data for each contract
    type CompilerContract: CompilerContract;
    /// Source parser used for resolving imports and version requirements.
    type ParsedSource: ParsedSource<Language = Self::Language>;
    /// Compiler settings.
    type Settings: CompilerSettings;
    /// Enum of languages supported by the compiler.
    type Language: Language;

    /// Main entrypoint for the compiler. Compiles given input into [CompilerOutput]. Takes
    /// ownership over the input and returns back version with potential modifications made to it.
    /// Returned input is always the one which was seen by the binary.
    fn compile(
        &self,
        input: &Self::Input,
    ) -> Result<CompilerOutput<Self::CompilationError, Self::CompilerContract>>;

    /// Returns all versions available locally and remotely. Should return versions with stripped
    /// metadata.
    fn available_versions(&self, language: &Self::Language) -> Vec<CompilerVersion>;
}

pub(crate) fn cache_version(
    path: PathBuf,
    args: &[String],
    f: impl FnOnce(&Path) -> Result<Version>,
) -> Result<Version> {
    #[allow(clippy::complexity)]
    static VERSION_CACHE: OnceLock<Mutex<HashMap<PathBuf, HashMap<Vec<String>, Version>>>> =
        OnceLock::new();
    let mut lock = VERSION_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(version) = lock.get(&path).and_then(|versions| versions.get(args)) {
        return Ok(version.clone());
    }

    let version = f(&path)?;

    lock.entry(path).or_default().insert(args.to_vec(), version.clone());

    Ok(version)
}
