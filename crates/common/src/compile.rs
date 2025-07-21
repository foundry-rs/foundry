//! Support for compiling [foundry_compilers::Project]
use crate::{
    preprocessor::TestOptimizerPreprocessor,
    reports::{report_kind, ReportKind},
    shell,
    term::SpinnerReporter,
    TestFunctionExt,
};
use alloy_json_abi::JsonAbi;
use alloy_primitives::hex;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, Cell, Color, Table};
use eyre::{eyre, ContextCompat, Result, WrapErr};
use fluentbase_build::{execute_build, Artifact as FluentArtifact, BuildArgs, BuildResult};
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    artifacts::{remappings::Remapping, BytecodeObject, Contract, Evm, Source, SourceFile},
    compilers::{
        solc::{Solc, SolcCompiler},
        Compiler,
    },
    info::ContractInfo as CompilerContractInfo,
    project::Preprocessor,
    report::{BasicStdoutReporter, NoReporter, Report},
    solc::SolcSettings,
    sources::VersionedSourceFile,
    AggregatedCompilerOutput, Artifact, Project, ProjectBuilder, ProjectCompileOutput,
    ProjectPathsConfig, SolcConfig,
};
use num_format::{Locale, ToFormattedString};
use std::{
    collections::BTreeMap,
    fmt::Display,
    io::IsTerminal,
    path::{Path, PathBuf},
    str::FromStr,
    time::Instant,
};

/// Builder type to configure how to compile a project.
///
/// This is merely a wrapper for [`Project::compile()`] which also prints to stdout depending on its
/// settings.
#[must_use = "ProjectCompiler does nothing unless you call a `compile*` method"]
pub struct ProjectCompiler {
    /// The root of the project.
    project_root: PathBuf,

    /// Whether we are going to verify the contracts after compilation.
    verify: Option<bool>,

    /// Whether to also print contract names.
    print_names: Option<bool>,

    /// Whether to also print contract sizes.
    print_sizes: Option<bool>,

    /// Whether to print anything at all. Overrides other `print` options.
    quiet: Option<bool>,

    /// Whether to bail on compiler errors.
    bail: Option<bool>,

    /// Whether to ignore the contract initcode size limit introduced by EIP-3860.
    ignore_eip_3860: bool,

    /// Extra files to include, that are not necessarily in the project's source dir.
    files: Vec<PathBuf>,

    /// Whether to compile with dynamic linking tests and scripts.
    dynamic_test_linking: bool,
}

impl Default for ProjectCompiler {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectCompiler {
    /// Create a new builder with the default settings.
    #[inline]
    pub fn new() -> Self {
        Self {
            project_root: PathBuf::new(),
            verify: None,
            print_names: None,
            print_sizes: None,
            quiet: Some(crate::shell::is_quiet()),
            bail: None,
            ignore_eip_3860: false,
            files: Vec::new(),
            dynamic_test_linking: false,
        }
    }

    /// Sets whether we are going to verify the contracts after compilation.
    #[inline]
    pub fn verify(mut self, yes: bool) -> Self {
        self.verify = Some(yes);
        self
    }

    /// Sets whether to print contract names.
    #[inline]
    pub fn print_names(mut self, yes: bool) -> Self {
        self.print_names = Some(yes);
        self
    }

    /// Sets whether to print contract sizes.
    #[inline]
    pub fn print_sizes(mut self, yes: bool) -> Self {
        self.print_sizes = Some(yes);
        self
    }

    /// Sets whether to print anything at all. Overrides other `print` options.
    #[inline]
    #[doc(alias = "silent")]
    pub fn quiet(mut self, yes: bool) -> Self {
        self.quiet = Some(yes);
        self
    }

    /// Sets whether to bail on compiler errors.
    #[inline]
    pub fn bail(mut self, yes: bool) -> Self {
        self.bail = Some(yes);
        self
    }

    /// Sets whether to ignore EIP-3860 initcode size limits.
    #[inline]
    pub fn ignore_eip_3860(mut self, yes: bool) -> Self {
        self.ignore_eip_3860 = yes;
        self
    }

    /// Sets extra files to include, that are not necessarily in the project's source dir.
    #[inline]
    pub fn files(mut self, files: impl IntoIterator<Item = PathBuf>) -> Self {
        self.files.extend(files);
        self
    }

    /// Sets if tests should be dynamically linked.
    #[inline]
    pub fn dynamic_test_linking(mut self, preprocess: bool) -> Self {
        self.dynamic_test_linking = preprocess;
        self
    }
    /// Compiles the project.
    pub fn compile<C: Compiler<CompilerContract = Contract>>(
        mut self,
        project: &Project<C>,
    ) -> Result<ProjectCompileOutput<C>>
    where
        TestOptimizerPreprocessor: Preprocessor<C>,
    {
        self.project_root = project.root().to_path_buf();

        // TODO: Avoid using std::process::exit(0).
        // Replacing this with a return (e.g., Ok(ProjectCompileOutput::default())) would be more
        // idiomatic, but it currently requires a `Default` bound on `C::Language`, which
        // breaks compatibility with downstream crates like `foundry-cli`. This would need a
        // broader refactor across the call chain. Leaving it as-is for now until a larger
        // refactor is feasible.
        if !project.paths.has_input_files() && self.files.is_empty() {
            sh_println!("Nothing to compile")?;
            std::process::exit(0);
        }

        // Taking is fine since we don't need these in `compile_with`.
        let files = std::mem::take(&mut self.files);
        let preprocess = self.dynamic_test_linking;

        let rust_artifacts = self.build_rust_contracts(&project)?;
        let mut output = self.compile_with(|| {
            let sources = if !files.is_empty() {
                Source::read_all(files)?
            } else {
                project.paths.read_input_files()?
            };

            let mut compiler =
                foundry_compilers::project::ProjectCompiler::with_sources(project, sources)?;
            if preprocess {
                compiler = compiler.with_preprocessor(TestOptimizerPreprocessor);
            }
            compiler.compile().map_err(Into::into)
        })?;
        // todo!()
        // self.integrate_rust_contracts_into_output(&mut output, rust_artifacts, project)?;

        Ok(output)
    }

    /// Compiles the project with the given closure
    ///
    /// # Example
    ///
    /// ```ignore
    /// use foundry_common::compile::ProjectCompiler;
    /// let config = foundry_config::Config::load().unwrap();
    /// let prj = config.project().unwrap();
    /// ProjectCompiler::new().compile_with(|| Ok(prj.compile()?)).unwrap();
    /// ```
    #[instrument(target = "forge::compile", skip_all)]
    fn compile_with<C: Compiler<CompilerContract = Contract>, F>(
        &self,
        f: F,
    ) -> Result<ProjectCompileOutput<C>>
    where
        F: FnOnce() -> Result<ProjectCompileOutput<C>>,
    {
        let quiet = self.quiet.unwrap_or(false);
        let bail = self.bail.unwrap_or(true);

        let output = with_compilation_reporter(quiet, || {
            tracing::debug!("compiling project");

            let timer = Instant::now();
            let r = f();
            let elapsed = timer.elapsed();

            tracing::debug!("finished compiling in {:.3}s", elapsed.as_secs_f64());
            r
        })?;

        if bail && output.has_compiler_errors() {
            eyre::bail!("{output}")
        }

        if !quiet {
            if !shell::is_json() {
                if output.is_unchanged() {
                    sh_println!("No files changed, compilation skipped")?;
                } else {
                    // print the compiler output / warnings
                    sh_println!("{output}")?;
                }
            }

            self.handle_output(&output)?;
        }

        Ok(output)
    }

    /// Finds and compiles Rust contracts, returning aggregated compilation output.
    fn build_rust_contracts<C: Compiler<CompilerContract = Contract>>(
        &self,
        project: &Project<C>,
    ) -> Result<Vec<(String, PathBuf, Contract)>> {
        // Find all Rust projects (crates) in source directories
        let rust_projects = find_rust_projects(&project.paths.sources)?;

        if rust_projects.is_empty() {
            sh_println!("No Rust contracts found")?;
            return Ok(Vec::new());
        }

        sh_println!("Compiling {} Rust contract(s)...", rust_projects.len())?;
        let timer = Instant::now();

        let mut artifacts = Vec::with_capacity(rust_projects.len());

        // Iterate through each found Rust project and compile it
        for rust_project_path in rust_projects {
            let contract_dir = rust_project_path
                .file_name()
                .ok_or_else(|| {
                    eyre!("Rust project path has no name: {}", rust_project_path.display())
                })?
                .to_str()
                .ok_or_else(|| eyre!("Rust project path is not valid UTF-8"))?;

            let contract_name_clean = Self::normalize_contract_name(contract_dir);
            let contract_name = format!("{contract_name_clean}.wasm");

            sh_println!("  - Compiling contract {contract_dir}:{contract_name}...");

            // Configure the build to generate Foundry artifact
            let build_args = BuildArgs {
                contract_name: Some(contract_name_clean.clone()),
                generate: vec![
                    FluentArtifact::Solidity,
                    FluentArtifact::Abi,
                    FluentArtifact::Foundry,
                    FluentArtifact::Rwasm,
                ],
                docker: false,
                output: Some(project.artifacts_path().to_path_buf()),
                ..Default::default()
            };

            sh_println!("  - Build args: {build_args:?}");

            // Execute Rust contract build
            let build_result = execute_build(&build_args, Some(rust_project_path.clone()))
                .map_err(|e| eyre::eyre!("Build failed: {}", e))
                .wrap_err_with(|| {
                    format!("Failed to build Rust contract at {}", rust_project_path.display())
                })?;

            // Read the generated Foundry artifact (foundry.json)
            let foundry_artifact_path = build_result
                .foundry_metadata_path
                .as_ref()
                .ok_or_else(|| eyre!("Foundry artifact was not generated for {}", contract_name))?;

            let foundry_artifact_json = std::fs::read_to_string(foundry_artifact_path)
                .wrap_err_with(|| {
                    format!(
                        "Failed to read Foundry artifact at {}",
                        foundry_artifact_path.display()
                    )
                })?;

            // Parse the Foundry artifact to extract the Contract
            let foundry_artifact: serde_json::Value = serde_json::from_str(&foundry_artifact_json)
                .wrap_err("Failed to parse Foundry artifact JSON")?;

            // Extract ABI and create Contract structure
            let abi: JsonAbi = serde_json::from_value(foundry_artifact["abi"].clone())
                .wrap_err("Failed to parse ABI from Foundry artifact")?;

            // Extract bytecode from Foundry artifact
            let bytecode_hex = foundry_artifact["bytecode"]["object"]
                .as_str()
                .ok_or_else(|| eyre!("No bytecode found in Foundry artifact"))?;

            let bytecode_bytes = if bytecode_hex.starts_with("0x") {
                hex::decode(&bytecode_hex[2..])
            } else {
                hex::decode(bytecode_hex)
            }
            .wrap_err("Failed to decode bytecode hex")?;

            let bytecode = BytecodeObject::Bytecode(bytecode_bytes.into());
            let evm = Evm {
                bytecode: Some(bytecode.clone().into()),
                deployed_bytecode: None,
                method_identifiers: BTreeMap::new(),
                assembly: None,
                legacy_assembly: None,
                gas_estimates: None,
            };

            let contract = Contract {
                abi: Some(abi),
                evm: Some(evm),
                userdoc: Default::default(),
                devdoc: Default::default(),
                storage_layout: Default::default(),
                transient_storage_layout: Default::default(),
                metadata: None,
                ir: None,
                ewasm: None,
                ir_optimized: None,
                ir_optimized_ast: None,
            };

            artifacts.push((contract_name_clean.clone(), rust_project_path, contract));

            // Save Foundry artifact with the correct name that Foundry expects
            // Create the contract-specific directory (e.g., out/PowerCalculator.wasm/)
            let contract_output_dir = project.artifacts_path().join(&contract_name);
            std::fs::create_dir_all(&contract_output_dir).wrap_err_with(|| {
                format!("Failed to create directory {}", contract_output_dir.display())
            })?;

            // Save as ContractName.json (what Foundry expects)
            let foundry_artifact_file =
                contract_output_dir.join(format!("{}.json", contract_name_clean));
            std::fs::copy(foundry_artifact_path, &foundry_artifact_file).wrap_err_with(|| {
                format!("Failed to copy Foundry artifact to {}", foundry_artifact_file.display())
            })?;

            sh_println!(
                "  - Successfully compiled {} to {:?}",
                contract_name,
                build_result.wasm_path
            );
            sh_println!("  - Saved Foundry artifact to {}", foundry_artifact_file.display());
        }

        sh_println!("Finished compiling Rust contracts in {:.2?}", timer.elapsed())?;
        Ok(artifacts)
    }

    /// Normalizes contract directory name to PascalCase contract name
    fn normalize_contract_name(contract_dir: &str) -> String {
        contract_dir
            .split('-')
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    }
                }
            })
            .collect()
    }

    // /// Integrates Rust contract artifacts into ProjectCompileOutput
    // fn integrate_rust_contracts_into_output<C: Compiler<CompilerContract = Contract>>(
    //     &self,
    //     output: &mut ProjectCompileOutput<C>,
    //     rust_contracts: Vec<(String, PathBuf, Contract)>,
    //     project: &Project<C>,
    // ) -> Result<()> {
    //     for (contract_name, _rust_project_path, contract) in rust_contracts {
    //         let interface_path = project
    //             .artifacts_path()
    //             .join(format!("{}.wasm", contract_name))
    //             .join("interface.sol");

    //         let versioned_contract = foundry_compilers::contracts::VersionedContract {
    //             contract,
    //             version: semver::Version::new(1, 0, 0),
    //             build_id: format!("rust-{}", contract_name),
    //             profile: "rust".to_string(),
    //         };

    //         // output
    //         //     .output_mut()
    //         //     .contracts
    //         //     .0
    //         //     .entry(interface_path.clone())
    //         //     .or_default()
    //         //     .entry(contract_name.clone())
    //         //     .or_default()
    //         //     .push(versioned_contract);

    //         output.output_mut().extend(semver::Version::new(1, 0, 0), build_info, profile,
    // output);

    //         sh_println!(
    //             "  - Added Rust contract {} to compilation output (source: interface.sol)",
    //             contract_name
    //         )?;
    //     }

    //     Ok(())
    // }

    // fn integrate_rust_contracts_into_output<C: Compiler<CompilerContract = Contract>>(
    //     &self,
    //     output: &mut ProjectCompileOutput<C>,
    //     rust_contracts: Vec<(String, PathBuf, Contract)>,
    //     project: &Project<C>,
    // ) -> Result<()> {
    //     if rust_contracts.is_empty() {
    //         return Ok(());
    //     }
    // 
    //     // ОТЛАДОЧНЫЙ КОД - посмотрим что есть в существующих build contexts
    //     sh_println!("=== DEBUG: Existing build info ===");
    //     for (build_id, build_context) in output.builds() {
    //         sh_println!("Build ID: {}", build_id);
    //         sh_println!("Build Context: {:#?}", build_context);
    //         break; // смотрим только первый для примера
    //     }
    // 
    //     // Также посмотрим на build_infos в compiler output
    //     sh_println!("=== DEBUG: Compiler output build infos ===");
    //     for (i, build_info) in output.output().build_infos.iter().enumerate() {
    //         sh_println!("Build Info #{}: ID={}", i, build_info.id);
    //         sh_println!("Build Context: {:#?}", build_info.build_context);
    //         sh_println!(
    //             "Build Info Map keys: {:?}",
    //             build_info.build_info.keys().collect::<Vec<_>>()
    //         );
    // 
    //         // Выводим некоторые значения из build_info map
    //         for (key, value) in build_info.build_info.iter().take(5) {
    //             sh_println!("  {}: {}", key, value);
    //         }
    // 
    //         if i == 0 {
    //             break;
    //         } // смотрим только первый
    //     }
    // 
    //     sh_println!("=== Integrating {} Rust contracts ===", rust_contracts.len());
    // 
    //     // Ваш существующий код для создания rust_compiler_output...
    //     let mut rust_compiler_output = foundry_compilers::CompilerOutput {
    //         errors: Vec::new(),
    //         sources: std::collections::BTreeMap::new(),
    //         contracts: std::collections::BTreeMap::new(),
    //         metadata: std::collections::BTreeMap::new(),
    //     };
    // 
    //     for (contract_name, _rust_project_path, contract) in rust_contracts {
    //         let interface_path = project
    //             .artifacts_path()
    //             .join(format!("{}.wasm", contract_name))
    //             .join("interface.sol");
    // 
    //         // Добавляем контракт
    //         let mut contracts_for_file = std::collections::BTreeMap::new();
    //         contracts_for_file.insert(contract_name.clone(), contract);
    //         rust_compiler_output.contracts.insert(interface_path.clone(), contracts_for_file);
    // 
    //         // Добавляем source file если существует
    //         if interface_path.exists() {
    //             let source_file = SourceFile { id: 0, ast: None };
    //             rust_compiler_output.sources.insert(interface_path.clone(), source_file);
    //         }
    //     }
    // 
    //     // ХАКАЕМ: используем существующий build info как шаблон
    //     if let Some(existing_build_info) = output.output().build_infos.first() {
    //         sh_println!("=== Using existing build info as template ===");
    // 
    //         let mut rust_build_info = existing_build_info.clone();
    // 
    //         // Модифицируем только ID и некоторые поля
    //         rust_build_info.id = format!(
    //             "rust-contracts-{}",
    //             std::time::SystemTime::now()
    //                 .duration_since(std::time::UNIX_EPOCH)
    //                 .unwrap_or_default()
    //                 .as_secs()
    //         );
    // 
    //         // Модифицируем build_info map для Rust
    //         rust_build_info
    //             .build_info
    //             .insert("compiler".to_string(), serde_json::Value::String("rust".to_string()));
    //         rust_build_info
    //             .build_info
    //             .insert("profile".to_string(), serde_json::Value::String("rust".to_string()));
    // 
    //         // Используем extend
    //         output.output_mut().extend(
    //             semver::Version::new(1, 0, 0),
    //             rust_build_info,
    //             "rust",
    //             rust_compiler_output,
    //         );
    //     } else {
    //         sh_println!("=== No existing build info found, skipping Rust integration ===");
    //         return Ok(());
    //     }
    // 
    //     sh_println!("=== Rust contracts integration complete ===");
    //     Ok(())
    // }

    /// If configured, this will print sizes or names
    fn handle_output<C: Compiler<CompilerContract = Contract>>(
        &self,
        output: &ProjectCompileOutput<C>,
    ) -> Result<()> {
        let print_names = self.print_names.unwrap_or(false);
        let print_sizes = self.print_sizes.unwrap_or(false);

        // print any sizes or names
        if print_names {
            let mut artifacts: BTreeMap<_, Vec<_>> = BTreeMap::new();
            for (name, (_, version)) in output.versioned_artifacts() {
                artifacts.entry(version).or_default().push(name);
            }

            if shell::is_json() {
                sh_println!("{}", serde_json::to_string(&artifacts).unwrap())?;
            } else {
                for (version, names) in artifacts {
                    sh_println!(
                        "  compiler version: {}.{}.{}",
                        version.major,
                        version.minor,
                        version.patch
                    )?;
                    for name in names {
                        sh_println!("    - {name}")?;
                    }
                }
            }
        }

        if print_sizes {
            // add extra newline if names were already printed
            if print_names && !shell::is_json() {
                sh_println!()?;
            }

            let mut size_report =
                SizeReport { report_kind: report_kind(), contracts: BTreeMap::new() };

            let mut artifacts: BTreeMap<String, Vec<_>> = BTreeMap::new();
            for (id, artifact) in output.artifact_ids().filter(|(id, _)| {
                // filter out forge-std specific contracts
                !id.source.to_string_lossy().contains("/forge-std/src/")
            }) {
                artifacts.entry(id.name.clone()).or_default().push((id.source.clone(), artifact));
            }

            for (name, artifact_list) in artifacts {
                for (path, artifact) in &artifact_list {
                    let runtime_size = contract_size(*artifact, false).unwrap_or_default();
                    let init_size = contract_size(*artifact, true).unwrap_or_default();

                    let is_dev_contract = artifact
                        .abi
                        .as_ref()
                        .map(|abi| {
                            abi.functions().any(|f| {
                                f.test_function_kind().is_known()
                                    || matches!(f.name.as_str(), "IS_TEST" | "IS_SCRIPT")
                            })
                        })
                        .unwrap_or(false);

                    let unique_name = if artifact_list.len() > 1 {
                        format!(
                            "{} ({})",
                            name,
                            path.strip_prefix(&self.project_root).unwrap_or(path).display()
                        )
                    } else {
                        name.clone()
                    };

                    size_report.contracts.insert(
                        unique_name,
                        ContractInfo { runtime_size, init_size, is_dev_contract },
                    );
                }
            }

            sh_println!("{size_report}")?;

            eyre::ensure!(
                !size_report.exceeds_runtime_size_limit(),
                "some contracts exceed the runtime size limit \
                 (EIP-170: {CONTRACT_RUNTIME_SIZE_LIMIT} bytes)"
            );
            // Check size limits only if not ignoring EIP-3860
            eyre::ensure!(
                self.ignore_eip_3860 || !size_report.exceeds_initcode_size_limit(),
                "some contracts exceed the initcode size limit \
                 (EIP-3860: {CONTRACT_INITCODE_SIZE_LIMIT} bytes)"
            );
        }

        Ok(())
    }
}

// https://eips.ethereum.org/EIPS/eip-170
const CONTRACT_RUNTIME_SIZE_LIMIT: usize = 24576;

// https://eips.ethereum.org/EIPS/eip-3860
const CONTRACT_INITCODE_SIZE_LIMIT: usize = 49152;

/// Contracts with info about their size
pub struct SizeReport {
    /// What kind of report to generate.
    report_kind: ReportKind,
    /// `contract name -> info`
    pub contracts: BTreeMap<String, ContractInfo>,
}

impl SizeReport {
    /// Returns the maximum runtime code size, excluding dev contracts.
    pub fn max_runtime_size(&self) -> usize {
        self.contracts
            .values()
            .filter(|c| !c.is_dev_contract)
            .map(|c| c.runtime_size)
            .max()
            .unwrap_or(0)
    }

    /// Returns the maximum initcode size, excluding dev contracts.
    pub fn max_init_size(&self) -> usize {
        self.contracts
            .values()
            .filter(|c| !c.is_dev_contract)
            .map(|c| c.init_size)
            .max()
            .unwrap_or(0)
    }

    /// Returns true if any contract exceeds the runtime size limit, excluding dev contracts.
    pub fn exceeds_runtime_size_limit(&self) -> bool {
        self.max_runtime_size() > CONTRACT_RUNTIME_SIZE_LIMIT
    }

    /// Returns true if any contract exceeds the initcode size limit, excluding dev contracts.
    pub fn exceeds_initcode_size_limit(&self) -> bool {
        self.max_init_size() > CONTRACT_INITCODE_SIZE_LIMIT
    }
}

impl Display for SizeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.report_kind {
            ReportKind::Text => {
                writeln!(f, "\n{}", self.format_table_output())?;
            }
            ReportKind::JSON => {
                writeln!(f, "{}", self.format_json_output())?;
            }
        }

        Ok(())
    }
}

impl SizeReport {
    fn format_json_output(&self) -> String {
        let contracts = self
                .contracts
                .iter()
                .filter(|(_, c)| !c.is_dev_contract && (c.runtime_size > 0 || c.init_size > 0))
                .map(|(name, contract)| {
                    (
                        name.clone(),
                        serde_json::json!({
                        "runtime_size": contract.runtime_size,
                        "init_size": contract.init_size,
                        "runtime_margin": CONTRACT_RUNTIME_SIZE_LIMIT as isize - contract.runtime_size as isize,
                        "init_margin": CONTRACT_INITCODE_SIZE_LIMIT as isize - contract.init_size as isize,
                    }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();

        serde_json::to_string(&contracts).unwrap()
    }

    fn format_table_output(&self) -> Table {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);

        table.set_header(vec![
            Cell::new("Contract"),
            Cell::new("Runtime Size (B)"),
            Cell::new("Initcode Size (B)"),
            Cell::new("Runtime Margin (B)"),
            Cell::new("Initcode Margin (B)"),
        ]);

        // Filters out dev contracts (Test or Script)
        let contracts = self
            .contracts
            .iter()
            .filter(|(_, c)| !c.is_dev_contract && (c.runtime_size > 0 || c.init_size > 0));
        for (name, contract) in contracts {
            let runtime_margin =
                CONTRACT_RUNTIME_SIZE_LIMIT as isize - contract.runtime_size as isize;
            let init_margin = CONTRACT_INITCODE_SIZE_LIMIT as isize - contract.init_size as isize;

            let runtime_color = match contract.runtime_size {
                ..18_000 => Color::Reset,
                18_000..=CONTRACT_RUNTIME_SIZE_LIMIT => Color::Yellow,
                _ => Color::Red,
            };

            let init_color = match contract.init_size {
                ..36_000 => Color::Reset,
                36_000..=CONTRACT_INITCODE_SIZE_LIMIT => Color::Yellow,
                _ => Color::Red,
            };

            let locale = &Locale::en;
            table.add_row([
                Cell::new(name),
                Cell::new(contract.runtime_size.to_formatted_string(locale)).fg(runtime_color),
                Cell::new(contract.init_size.to_formatted_string(locale)).fg(init_color),
                Cell::new(runtime_margin.to_formatted_string(locale)).fg(runtime_color),
                Cell::new(init_margin.to_formatted_string(locale)).fg(init_color),
            ]);
        }

        table
    }
}

/// Returns the deployed or init size of the contract.
fn contract_size<T: Artifact>(artifact: &T, initcode: bool) -> Option<usize> {
    let bytecode = if initcode {
        artifact.get_bytecode_object()?
    } else {
        artifact.get_deployed_bytecode_object()?
    };

    let size = match bytecode.as_ref() {
        BytecodeObject::Bytecode(bytes) => bytes.len(),
        BytecodeObject::Unlinked(unlinked) => {
            // we don't need to account for placeholders here, because library placeholders take up
            // 40 characters: `__$<library hash>$__` which is the same as a 20byte address in hex.
            let mut size = unlinked.len();
            if unlinked.starts_with("0x") {
                size -= 2;
            }
            // hex -> bytes
            size / 2
        }
    };

    Some(size)
}

/// How big the contract is and whether it is a dev contract where size limits can be neglected
#[derive(Clone, Copy, Debug)]
pub struct ContractInfo {
    /// Size of the runtime code in bytes
    pub runtime_size: usize,
    /// Size of the initcode in bytes
    pub init_size: usize,
    /// A development contract is either a Script or a Test contract.
    pub is_dev_contract: bool,
}

/// Compiles target file path.
///
/// If `quiet` no solc related output will be emitted to stdout.
///
/// If `verify` and it's a standalone script, throw error. Only allowed for projects.
///
/// **Note:** this expects the `target_path` to be absolute
pub fn compile_target<C: Compiler<CompilerContract = Contract>>(
    target_path: &Path,
    project: &Project<C>,
    quiet: bool,
) -> Result<ProjectCompileOutput<C>>
where
    TestOptimizerPreprocessor: Preprocessor<C>,
{
    ProjectCompiler::new().quiet(quiet).files([target_path.into()]).compile(project)
}

/// Creates a [Project] from an Etherscan source.
pub fn etherscan_project(
    metadata: &Metadata,
    target_path: impl AsRef<Path>,
) -> Result<Project<SolcCompiler>> {
    let target_path = dunce::canonicalize(target_path.as_ref())?;
    let sources_path = target_path.join(&metadata.contract_name);
    metadata.source_tree().write_to(&target_path)?;

    let mut settings = metadata.settings()?;

    // make remappings absolute with our root
    for remapping in &mut settings.remappings {
        let new_path = sources_path.join(remapping.path.trim_start_matches('/'));
        remapping.path = new_path.display().to_string();
    }

    // add missing remappings
    if !settings.remappings.iter().any(|remapping| remapping.name.starts_with("@openzeppelin/")) {
        let oz = Remapping {
            context: None,
            name: "@openzeppelin/".into(),
            path: sources_path.join("@openzeppelin").display().to_string(),
        };
        settings.remappings.push(oz);
    }

    // root/
    //   ContractName/
    //     [source code]
    let paths = ProjectPathsConfig::builder()
        .sources(sources_path.clone())
        .remappings(settings.remappings.clone())
        .build_with_root(sources_path);

    let v = metadata.compiler_version()?;
    let solc = Solc::find_or_install(&v)?;

    let compiler = SolcCompiler::Specific(solc);

    Ok(ProjectBuilder::<SolcCompiler>::default()
        .settings(SolcSettings {
            settings: SolcConfig::builder().settings(settings).build(),
            ..Default::default()
        })
        .paths(paths)
        .ephemeral()
        .no_artifacts()
        .build(compiler)?)
}

/// Configures the reporter and runs the given closure.
pub fn with_compilation_reporter<O>(quiet: bool, f: impl FnOnce() -> O) -> O {
    #[expect(clippy::collapsible_else_if)]
    let reporter = if quiet || shell::is_json() {
        Report::new(NoReporter::default())
    } else {
        if std::io::stdout().is_terminal() {
            Report::new(SpinnerReporter::spawn())
        } else {
            Report::new(BasicStdoutReporter::default())
        }
    };

    foundry_compilers::report::with_scoped(&reporter, f)
}

fn find_rust_projects(src_root: &Path) -> Result<Vec<PathBuf>> {
    sh_println!("find_rust_projects at: src_root={}", src_root.display());
    let mut projects = Vec::new();
    if !src_root.is_dir() {
        return Ok(projects);
    }

    for entry in std::fs::read_dir(src_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("Cargo.toml").exists() {
            projects.push(path);
        }
    }

    Ok(projects)
}

/// Container type for parsing contract identifiers from CLI.
///
/// Passed string can be of the following forms:
/// - `src/Counter.sol` - path to the contract file, in the case where it only contains one contract
/// - `src/Counter.sol:Counter` - path to the contract file and the contract name
/// - `Counter` - contract name only
#[derive(Clone, PartialEq, Eq)]
pub enum PathOrContractInfo {
    /// Non-canoncalized path provided via CLI.
    Path(PathBuf),
    /// Contract info provided via CLI.
    ContractInfo(CompilerContractInfo),
}

impl PathOrContractInfo {
    /// Returns the path to the contract file if provided.
    pub fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) => Some(path.to_path_buf()),
            Self::ContractInfo(info) => info.path.as_ref().map(PathBuf::from),
        }
    }

    /// Returns the contract name if provided.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Path(_) => None,
            Self::ContractInfo(info) => Some(&info.name),
        }
    }
}

impl FromStr for PathOrContractInfo {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self> {
        if let Ok(contract) = CompilerContractInfo::from_str(s) {
            return Ok(Self::ContractInfo(contract));
        }
        let path = PathBuf::from(s);
        if path.extension().is_some_and(|ext| ext == "sol" || ext == "vy") {
            return Ok(Self::Path(path));
        }
        Err(eyre::eyre!("Invalid contract identifier, file is not *.sol or *.vy: {}", s))
    }
}

impl std::fmt::Debug for PathOrContractInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => write!(f, "Path({})", path.display()),
            Self::ContractInfo(info) => {
                write!(f, "ContractInfo({info})")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_contract_identifiers() {
        let t = ["src/Counter.sol", "src/Counter.sol:Counter", "Counter"];

        let i1 = PathOrContractInfo::from_str(t[0]).unwrap();
        assert_eq!(i1, PathOrContractInfo::Path(PathBuf::from(t[0])));

        let i2 = PathOrContractInfo::from_str(t[1]).unwrap();
        assert_eq!(
            i2,
            PathOrContractInfo::ContractInfo(CompilerContractInfo {
                path: Some("src/Counter.sol".to_string()),
                name: "Counter".to_string()
            })
        );

        let i3 = PathOrContractInfo::from_str(t[2]).unwrap();
        assert_eq!(
            i3,
            PathOrContractInfo::ContractInfo(CompilerContractInfo {
                path: None,
                name: "Counter".to_string()
            })
        );
    }
}
