use foundry_compilers::{
    artifacts::{
        error::SourceLocation, output_selection::OutputSelection, sources::Source, Contract, EvmVersion,
        Severity, Sources,
    }, solc::Restriction, CompilationError, Compiler, CompilerInput,
    CompilerOutput, CompilerSettings, CompilerSettingsRestrictions, CompilerVersion, Language,
    ParsedSource,
    ProjectPathsConfig,
};
use semver::{Version, VersionReq};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt,
    fmt::Display,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RustWasmLanguage;

impl serde::Serialize for RustWasmLanguage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("rust")
    }
}

impl<'de> serde::Deserialize<'de> for RustWasmLanguage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let res = String::deserialize(deserializer)?;
        if res != "vyper" {
            Err(serde::de::Error::custom(format!("Invalid Vyper language: {res}")))
        } else {
            Ok(Self)
        }
    }
}

impl Language for RustWasmLanguage {
    const FILE_EXTENSIONS: &'static [&'static str] = &["rs"];
}

impl Display for RustWasmLanguage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "RustWasm")
    }
}

// --- 2. Compilation Error ---
// A minimal error type to satisfy the `CompilationError` trait.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RustWasmCompilationError {
    pub message: String,
}

impl CompilationError for RustWasmCompilationError {
    fn is_warning(&self) -> bool {
        false // Assuming all our compiler errors are critical
    }
    fn is_error(&self) -> bool {
        true
    }
    fn source_location(&self) -> Option<SourceLocation> {
        None // Can be improved to parse location from Cargo's output
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn error_code(&self) -> Option<u64> {
        None
    }
}

impl Display for RustWasmCompilationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

// --- 3. Compiler Settings ---
// Minimal settings struct. For Rust, most settings are in `Cargo.toml`.
// We can add fields here if we need to pass specific command-line args.

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RustWasmSettings;

impl CompilerSettings for RustWasmSettings {
    type Restrictions = RustRestrictions;
    fn update_output_selection(&mut self, _f: impl FnOnce(&mut OutputSelection)) {
        // Not applicable to Rust/WASM build process
        unimplemented!()
    }
    fn can_use_cached(&self, _other: &Self) -> bool {
        // For simplicity, we can always recompile. Caching can be implemented later.
        false
    }
    fn with_include_paths(self, _include_paths: &std::collections::BTreeSet<PathBuf>) -> Self {
        self // Not applicable
    }
    fn satisfies_restrictions(&self, _restrictions: &Self::Restrictions) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RustRestrictions {
    pub evm_version: Restriction<EvmVersion>,
}

impl CompilerSettingsRestrictions for RustRestrictions {
    fn merge(self, other: Self) -> Option<Self> {
        Some(Self { evm_version: self.evm_version.merge(other.evm_version)? })
    }
}

// --- 4. Source Parser ---
// Minimal implementation for `ParsedSource`. Rust dependencies are managed
// by Cargo, so we don't need to parse import paths from source files.

#[derive(Clone, Debug)]
pub struct RustWasmParsedSource {
    path: PathBuf,
}

impl ParsedSource for RustWasmParsedSource {
    type Language = RustWasmLanguage;

    fn parse(content: &str, file: &Path) -> foundry_compilers::error::Result<Self> {
        Ok(Self { path: file.to_path_buf() })
    }
    fn version_req(&self) -> Option<&VersionReq> {
        None // Version is managed by the Rust toolchain, not in-file pragmas
    }
    fn contract_names(&self) -> &[String] {
        &[] // The contract name comes from the Cargo package name
    }
    fn language(&self) -> Self::Language {
        RustWasmLanguage
    }
    fn resolve_imports<C>(
        &self,
        _paths: &ProjectPathsConfig<C>,
        _include_paths: &mut std::collections::BTreeSet<PathBuf>,
    ) -> foundry_compilers::error::Result<Vec<PathBuf>> {
        Ok(Vec::new()) // Cargo handles all dependency resolution
    }
}

// --- 5. Compiler Input ---
// The input struct that holds sources and settings, similar to `VyperVersionedInput`.

#[derive(Clone, Debug, serde::Serialize)]
pub struct RustWasmInput {
    pub sources: Sources,
    pub settings: RustWasmSettings,
    #[serde(skip)]
    pub version: Version,
}

impl foundry_compilers::CompilerInput for RustWasmInput {
    type Settings = RustWasmSettings;
    type Language = RustWasmLanguage;

    fn build(
        sources: Sources,
        settings: Self::Settings,
        _language: Self::Language,
        version: Version,
    ) -> Self {
        Self { sources, settings, version }
    }
    fn compiler_name(&self) -> Cow<'static, str> {
        "RustWasmCompiler".into()
    }
    fn strip_prefix(&mut self, base: &Path) {
        self.sources = self
            .sources
            .iter()
            .map(|(path, source)| {
                (path.strip_prefix(base).unwrap_or(path).to_path_buf(), source.clone())
            })
            .collect();
    }
    fn language(&self) -> Self::Language {
        RustWasmLanguage
    }
    fn version(&self) -> &Version {
        &self.version
    }
    fn sources(&self) -> impl Iterator<Item = (&Path, &Source)> {
        self.sources.iter().map(|(path, source)| (path.as_path(), source))
    }
}

// --- 6. The Compiler ---
// The main compiler struct. It's stateless in this minimal implementation.

#[derive(Debug, Clone, Default)]
pub struct RustWasmCompiler;

impl RustWasmCompiler {
    /// A helper function to find the root of a Cargo project from a source file.
    fn find_project_root(path: &Path) -> Option<PathBuf> {
        let mut current = path.parent()?;
        loop {
            if current.join("Cargo.toml").exists() {
                return Some(current.to_path_buf());
            }
            current = current.parent()?;
        }
    }

    /// A helper to normalize contract directory name to PascalCase contract name.
    /// This is taken directly from your example.
    fn normalize_contract_name(contract_dir: &str) -> String {
        contract_dir
            .split('-')
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect()
    }

    /// A helper that encapsulates the core build logic for a single Rust project.
    /// This is where the logic from your `build_rust_contracts` function is adapted.
    fn compile_rust_project(
        project_path: &Path,
    ) -> std::result::Result<(String, Contract), String> {
        // This is a placeholder for your actual build logic using your internal builder.
        // For this example, we'll simulate a successful build that produces a `Contract`.
        // Replace the following simulation with your `execute_build` call.

        let contract_dir_name = project_path.file_name().unwrap().to_str().unwrap();
        let contract_name = Self::normalize_contract_name(contract_dir_name);

        println!("    - Compiling project: {}", project_path.display());

        // --- START: Replace this with your actual build logic ---
        // Example:
        // let build_args = BuildArgs { ... };
        // let build_result = execute_build(&build_args, Some(project_path.to_path_buf()))
        //     .map_err(|e| e.to_string())?;
        // let foundry_artifact_path = build_result.foundry_metadata_path.unwrap();
        // let contract_json = std::fs::read_to_string(foundry_artifact_path).unwrap();
        // let contract: Contract = serde_json::from_str(&contract_json).unwrap();
        // --- END: Replacement block ---

        // Simulating a successful build for demonstration
        let contract = Contract {
            abi: None,
            metadata: None,
            userdoc: Default::default(),
            devdoc: Default::default(),
            ir: None,
            storage_layout: Default::default(),
            transient_storage_layout: Default::default(),
            evm: None,
            ewasm: None,
            ir_optimized: None,
            ir_optimized_ast: None,
        }; // In reality, this is parsed from foundry.json
        println!("    - Successfully compiled contract: {}", contract_name);

        Ok((contract_name, contract))
    }
}

impl Compiler for RustWasmCompiler {
    type Settings = RustWasmSettings;
    type CompilationError = RustWasmCompilationError;
    type ParsedSource = RustWasmParsedSource;
    type Input = RustWasmInput;
    type Language = RustWasmLanguage;
    type CompilerContract = Contract;

    /// Returns the available version of the compiler. For Rust, this could be
    /// the version of `cargo` or your internal builder. We'll hardcode it here.
    fn available_versions(&self, _language: &Self::Language) -> Vec<CompilerVersion> {
        // This should reflect the version of your rust-wasm builder toolchain.
        vec![CompilerVersion::Installed(Version::new(0, 1, 0))]
    }

    /// The main compilation entrypoint.
    fn compile(
        &self,
        input: &Self::Input,
    ) -> foundry_compilers::error::Result<
        CompilerOutput<Self::CompilationError, Self::CompilerContract>,
    > {
        let mut errors = Vec::new();
        let mut contracts = BTreeMap::new();
        let mut project_to_sources: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();

        // 1. Group source files by their Cargo project root.
        for path in input.sources.keys() {
            if let Some(root) = Self::find_project_root(path) {
                project_to_sources.entry(root).or_default().push(path.clone());
            } else {
                errors.push(RustWasmCompilationError {
                    message: format!(
                        "Could not find Cargo.toml for source file: {}",
                        path.display()
                    ),
                });
            }
        }

        // // 2. Compile each unique project once.
        // for (project_path, source_files) in project_to_sources {
        //     match Self::compile_rust_project(&project_path) {
        //         Ok((contract_name, contract)) => {
        //             // 3. Associate the compiled artifact with each source file from that
        // project.             for source_path in source_files {
        //                 let file_contracts:: &mut V = contracts.entry(source_path).or_default();
        //                 file_contracts.insert(contract_name.clone(), contract.clone());
        //             }
        //         }
        //         Err(err) => {
        //             errors.push(RustWasmCompilationError { message: err });
        //         }
        //     }
        // }

        // 4. Construct the final output.
        Ok(CompilerOutput {
            errors,
            contracts,
            sources: BTreeMap::default(),
            metadata: Default::default(),
        })
    }
}
