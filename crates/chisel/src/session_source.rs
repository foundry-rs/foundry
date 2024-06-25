//! Session Source
//!
//! This module contains the `SessionSource` struct, which is a minimal wrapper around
//! the REPL contract's source code. It provides simple compilation, parsing, and
//! execution helpers.

use eyre::Result;
use forge_fmt::solang_ext::SafeUnwrap;
use foundry_compilers::{
    artifacts::{CompilerOutput, Settings, SolcInput, Source, Sources},
    compilers::solc::Solc,
};
use foundry_config::{Config, SolcReq};
use foundry_evm::{backend::Backend, opts::EvmOpts};
use semver::Version;
use serde::{Deserialize, Serialize};
use solang_parser::{diagnostics::Diagnostic, pt};
use std::{collections::HashMap, fs, path::PathBuf};
use yansi::Paint;

/// The minimum Solidity version of the `Vm` interface.
pub const MIN_VM_VERSION: Version = Version::new(0, 6, 2);

/// Solidity source for the `Vm` interface in [forge-std](https://github.com/foundry-rs/forge-std)
static VM_SOURCE: &str = include_str!("../../../testdata/cheats/Vm.sol");

/// Intermediate output for the compiled [SessionSource]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntermediateOutput {
    /// All expressions within the REPL contract's run function and top level scope.
    #[serde(skip)]
    pub repl_contract_expressions: HashMap<String, pt::Expression>,
    /// Intermediate contracts
    #[serde(skip)]
    pub intermediate_contracts: IntermediateContracts,
}

/// A refined intermediate parse tree for a contract that enables easy lookups
/// of definitions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntermediateContract {
    /// All function definitions within the contract
    #[serde(skip)]
    pub function_definitions: HashMap<String, Box<pt::FunctionDefinition>>,
    /// All event definitions within the contract
    #[serde(skip)]
    pub event_definitions: HashMap<String, Box<pt::EventDefinition>>,
    /// All struct definitions within the contract
    #[serde(skip)]
    pub struct_definitions: HashMap<String, Box<pt::StructDefinition>>,
    /// All variable definitions within the top level scope of the contract
    #[serde(skip)]
    pub variable_definitions: HashMap<String, Box<pt::VariableDefinition>>,
}

/// A defined type for a map of contract names to [IntermediateContract]s
type IntermediateContracts = HashMap<String, IntermediateContract>;

/// Full compilation output for the [SessionSource]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedOutput {
    /// The [IntermediateOutput] component
    pub intermediate: IntermediateOutput,
    /// The [CompilerOutput] component
    pub compiler_output: CompilerOutput,
}

/// Configuration for the [SessionSource]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionSourceConfig {
    /// Foundry configuration
    pub foundry_config: Config,
    /// EVM Options
    pub evm_opts: EvmOpts,
    /// Disable the default `Vm` import.
    pub no_vm: bool,
    #[serde(skip)]
    /// In-memory REVM db for the session's runner.
    pub backend: Option<Backend>,
    /// Optionally enable traces for the REPL contract execution
    pub traces: bool,
    /// Optionally set calldata for the REPL contract execution
    pub calldata: Option<Vec<u8>>,
}

impl SessionSourceConfig {
    /// Returns the solc version to use
    ///
    /// Solc version precedence
    /// - Foundry configuration / `--use` flag
    /// - Latest installed version via SVM
    /// - Default: Latest 0.8.19
    pub(crate) fn solc(&self) -> Result<Solc> {
        let solc_req = if let Some(solc_req) = self.foundry_config.solc.clone() {
            solc_req
        } else if let Some(version) = Solc::installed_versions().into_iter().max() {
            SolcReq::Version(version)
        } else {
            if !self.foundry_config.offline {
                print!("{}", "No solidity versions installed! ".green());
            }
            // use default
            SolcReq::Version(Version::new(0, 8, 19))
        };

        match solc_req {
            SolcReq::Version(version) => {
                // Validate that the requested evm version is supported by the solc version
                let req_evm_version = self.foundry_config.evm_version;
                if let Some(compat_evm_version) = req_evm_version.normalize_version_solc(&version) {
                    if req_evm_version > compat_evm_version {
                        eyre::bail!(
                            "The set evm version, {req_evm_version}, is not supported by solc {version}. Upgrade to a newer solc version."
                        );
                    }
                }

                let solc = if let Some(solc) = Solc::find_svm_installed_version(&version)? {
                    solc
                } else {
                    if self.foundry_config.offline {
                        eyre::bail!("can't install missing solc {version} in offline mode")
                    }
                    println!("{}", format!("Installing solidity version {version}...").green());
                    Solc::blocking_install(&version)?
                };
                Ok(solc)
            }
            SolcReq::Local(solc) => {
                if !solc.is_file() {
                    eyre::bail!("`solc` {} does not exist", solc.display());
                }
                Ok(Solc::new(solc)?)
            }
        }
    }
}

/// REPL Session Source wrapper
///
/// Heavily based on soli's [`ConstructedSource`](https://github.com/jpopesculian/soli/blob/master/src/main.rs#L166)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSource {
    /// The file name
    pub file_name: PathBuf,
    /// The contract name
    pub contract_name: String,
    /// The solidity compiler version
    pub solc: Solc,
    /// Global level solidity code
    ///
    /// Typically, global-level code is present between the contract definition and the first
    /// function (usually constructor)
    pub global_code: String,
    /// Top level solidity code
    ///
    /// Typically, this is code seen above the constructor
    pub top_level_code: String,
    /// Code existing within the "run()" function's scope
    pub run_code: String,
    /// The generated output
    pub generated_output: Option<GeneratedOutput>,
    /// Session Source configuration
    pub config: SessionSourceConfig,
}

impl SessionSource {
    /// Creates a new source given a solidity compiler version
    ///
    /// # Panics
    ///
    /// If no Solc binary is set, cannot be found or the `--version` command fails
    ///
    /// ### Takes
    ///
    /// - An instance of [Solc]
    /// - An instance of [SessionSourceConfig]
    ///
    /// ### Returns
    ///
    /// A new instance of [SessionSource]
    #[track_caller]
    pub fn new(solc: Solc, mut config: SessionSourceConfig) -> Self {
        if solc.version < MIN_VM_VERSION && !config.no_vm {
            tracing::info!(version=%solc.version, minimum=%MIN_VM_VERSION, "Disabling VM injection");
            config.no_vm = true;
        }

        Self {
            file_name: PathBuf::from("ReplContract.sol".to_string()),
            contract_name: "REPL".to_string(),
            solc,
            config,
            global_code: Default::default(),
            top_level_code: Default::default(),
            run_code: Default::default(),
            generated_output: None,
        }
    }

    /// Clones a [SessionSource] without copying the [GeneratedOutput], as it will
    /// need to be regenerated as soon as new code is added.
    ///
    /// ### Returns
    ///
    /// A shallow-cloned [SessionSource]
    pub fn shallow_clone(&self) -> Self {
        Self {
            file_name: self.file_name.clone(),
            contract_name: self.contract_name.clone(),
            solc: self.solc.clone(),
            global_code: self.global_code.clone(),
            top_level_code: self.top_level_code.clone(),
            run_code: self.run_code.clone(),
            generated_output: None,
            config: self.config.clone(),
        }
    }

    /// Clones the [SessionSource] and appends a new line of code. Will return
    /// an error result if the new line fails to be parsed.
    ///
    /// ### Returns
    ///
    /// Optionally, a shallow-cloned [SessionSource] with the passed content appended to the
    /// source code.
    pub fn clone_with_new_line(&self, mut content: String) -> Result<(Self, bool)> {
        let new_source = self.shallow_clone();
        if let Some(parsed) = parse_fragment(new_source.solc, new_source.config, &content)
            .or_else(|| {
                let new_source = self.shallow_clone();
                content.push(';');
                parse_fragment(new_source.solc, new_source.config, &content)
            })
            .or_else(|| {
                let new_source = self.shallow_clone();
                content = content.trim_end().trim_end_matches(';').to_string();
                parse_fragment(new_source.solc, new_source.config, &content)
            })
        {
            let mut new_source = self.shallow_clone();
            // Flag that tells the dispatcher whether to build or execute the session
            // source based on the scope of the new code.
            match parsed {
                ParseTreeFragment::Function => new_source.with_run_code(&content),
                ParseTreeFragment::Contract => new_source.with_top_level_code(&content),
                ParseTreeFragment::Source => new_source.with_global_code(&content),
            };

            Ok((new_source, matches!(parsed, ParseTreeFragment::Function)))
        } else {
            eyre::bail!("\"{}\"", content.trim().to_owned());
        }
    }

    // Fillers

    /// Appends global-level code to the source
    pub fn with_global_code(&mut self, content: &str) -> &mut Self {
        self.global_code.push_str(content.trim());
        self.global_code.push('\n');
        self.generated_output = None;
        self
    }

    /// Appends top-level code to the source
    pub fn with_top_level_code(&mut self, content: &str) -> &mut Self {
        self.top_level_code.push_str(content.trim());
        self.top_level_code.push('\n');
        self.generated_output = None;
        self
    }

    /// Appends code to the "run()" function
    pub fn with_run_code(&mut self, content: &str) -> &mut Self {
        self.run_code.push_str(content.trim());
        self.run_code.push('\n');
        self.generated_output = None;
        self
    }

    // Drains

    /// Clears global code from the source
    pub fn drain_global_code(&mut self) -> &mut Self {
        String::clear(&mut self.global_code);
        self.generated_output = None;
        self
    }

    /// Clears top-level code from the source
    pub fn drain_top_level_code(&mut self) -> &mut Self {
        String::clear(&mut self.top_level_code);
        self.generated_output = None;
        self
    }

    /// Clears the "run()" function's code
    pub fn drain_run(&mut self) -> &mut Self {
        String::clear(&mut self.run_code);
        self.generated_output = None;
        self
    }

    /// Generates and [`SolcInput`] from the source.
    ///
    /// ### Returns
    ///
    /// A [`SolcInput`] object containing forge-std's `Vm` interface as well as the REPL contract
    /// source.
    pub fn compiler_input(&self) -> SolcInput {
        let mut sources = Sources::new();
        sources.insert(self.file_name.clone(), Source::new(self.to_repl_source()));

        let remappings = self.config.foundry_config.get_all_remappings().collect::<Vec<_>>();

        // Include Vm.sol if forge-std remapping is not available
        if !self.config.no_vm && !remappings.iter().any(|r| r.name.starts_with("forge-std")) {
            sources.insert(PathBuf::from("forge-std/Vm.sol"), Source::new(VM_SOURCE));
        }

        let settings = Settings {
            remappings,
            evm_version: Some(self.config.foundry_config.evm_version),
            ..Default::default()
        };

        // we only care about the solidity source, so we can safely unwrap
        SolcInput::resolve_and_build(sources, settings)
            .into_iter()
            .next()
            .map(|i| i.sanitized(&self.solc.version))
            .expect("Solidity source not found")
    }

    /// Compiles the source using [solang_parser]
    ///
    /// ### Returns
    ///
    /// A [pt::SourceUnit] if successful.
    /// A vec of [solang_parser::diagnostics::Diagnostic]s if unsuccessful.
    pub fn parse(&self) -> Result<pt::SourceUnit, Vec<solang_parser::diagnostics::Diagnostic>> {
        solang_parser::parse(&self.to_repl_source(), 0).map(|(pt, _)| pt)
    }

    /// Generate intermediate contracts for all contract definitions in the compilation source.
    ///
    /// ### Returns
    ///
    /// Optionally, a map of contract names to a vec of [IntermediateContract]s.
    pub fn generate_intermediate_contracts(&self) -> Result<HashMap<String, IntermediateContract>> {
        let mut res_map = HashMap::new();
        let parsed_map = self.compiler_input().sources;
        for source in parsed_map.values() {
            Self::get_intermediate_contract(&source.content, &mut res_map);
        }
        Ok(res_map)
    }

    /// Generate intermediate output for the REPL contract
    pub fn generate_intermediate_output(&self) -> Result<IntermediateOutput> {
        // Parse generate intermediate contracts
        let intermediate_contracts = self.generate_intermediate_contracts()?;

        // Construct variable definitions
        let variable_definitions = intermediate_contracts
            .get("REPL")
            .ok_or_else(|| eyre::eyre!("Could not find intermediate REPL contract!"))?
            .variable_definitions
            .clone()
            .into_iter()
            .map(|(k, v)| (k, v.ty))
            .collect::<HashMap<String, pt::Expression>>();
        // Construct intermediate output
        let mut intermediate_output = IntermediateOutput {
            repl_contract_expressions: variable_definitions,
            intermediate_contracts,
        };

        // Add all statements within the run function to the repl_contract_expressions map
        for (key, val) in intermediate_output
            .run_func_body()?
            .clone()
            .iter()
            .flat_map(Self::get_statement_definitions)
        {
            intermediate_output.repl_contract_expressions.insert(key, val);
        }

        Ok(intermediate_output)
    }

    /// Compile the contract
    ///
    /// ### Returns
    ///
    /// Optionally, a [CompilerOutput] object that contains compilation artifacts.
    pub fn compile(&self) -> Result<CompilerOutput> {
        // Compile the contract
        let compiled = self.solc.compile_exact(&self.compiler_input())?;

        // Extract compiler errors
        let errors =
            compiled.errors.iter().filter(|error| error.severity.is_error()).collect::<Vec<_>>();
        if !errors.is_empty() {
            eyre::bail!(
                "Compiler errors:\n{}",
                errors.into_iter().map(|err| err.to_string()).collect::<String>()
            );
        }

        Ok(compiled)
    }

    /// Builds the SessionSource from input into the complete CompiledOutput
    ///
    /// ### Returns
    ///
    /// Optionally, a [GeneratedOutput] object containing both the [CompilerOutput] and the
    /// [IntermediateOutput].
    pub fn build(&mut self) -> Result<GeneratedOutput> {
        // Compile
        let compiler_output = self.compile()?;

        // Generate intermediate output
        let intermediate_output = self.generate_intermediate_output()?;

        // Construct generated output
        let generated_output =
            GeneratedOutput { intermediate: intermediate_output, compiler_output };
        self.generated_output = Some(generated_output.clone()); // ehhh, need to not clone this.
        Ok(generated_output)
    }

    /// Convert the [SessionSource] to a valid Script contract
    ///
    /// ### Returns
    ///
    /// The [SessionSource] represented as a Forge Script contract.
    pub fn to_script_source(&self) -> String {
        let Version { major, minor, patch, .. } = self.solc.version;
        let Self { contract_name, global_code, top_level_code, run_code, config, .. } = self;

        let script_import =
            if !config.no_vm { "import {Script} from \"forge-std/Script.sol\";\n" } else { "" };

        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^{major}.{minor}.{patch};

{script_import}
{global_code}

contract {contract_name} is Script {{
    {top_level_code}
  
    /// @notice Script entry point
    function run() public {{
        {run_code}
    }}
}}"#,
        )
    }

    /// Convert the [SessionSource] to a valid REPL contract
    ///
    /// ### Returns
    ///
    /// The [SessionSource] represented as a REPL contract.
    pub fn to_repl_source(&self) -> String {
        let Version { major, minor, patch, .. } = self.solc.version;
        let Self { contract_name, global_code, top_level_code, run_code, config, .. } = self;

        let (vm_import, vm_constant) = if !config.no_vm {
            (
                "import {Vm} from \"forge-std/Vm.sol\";\n",
                "Vm internal constant vm = Vm(address(uint160(uint256(keccak256(\"hevm cheat code\")))));\n"
            )
        } else {
            ("", "")
        };

        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^{major}.{minor}.{patch};

{vm_import}
{global_code}

contract {contract_name} {{
    {vm_constant}
    {top_level_code}
  
    /// @notice REPL contract entry point
    function run() public {{
        {run_code}
    }}
}}"#,
        )
    }

    /// Gets the [IntermediateContract] for a Solidity source string and inserts it into the
    /// passed `res_map`. In addition, recurses on any imported files as well.
    ///
    /// ### Takes
    /// - `content` - A Solidity source string
    /// - `res_map` - A mutable reference to a map of contract names to [IntermediateContract]s
    pub fn get_intermediate_contract(
        content: &str,
        res_map: &mut HashMap<String, IntermediateContract>,
    ) {
        if let Ok((pt::SourceUnit(source_unit_parts), _)) = solang_parser::parse(content, 0) {
            let func_defs = source_unit_parts
                .into_iter()
                .filter_map(|sup| match sup {
                    pt::SourceUnitPart::ImportDirective(i) => match i {
                        pt::Import::Plain(s, _) |
                        pt::Import::Rename(s, _, _) |
                        pt::Import::GlobalSymbol(s, _, _) => {
                            let s = match s {
                                pt::ImportPath::Filename(s) => s.string,
                                pt::ImportPath::Path(p) => p.to_string(),
                            };
                            let path = PathBuf::from(s);

                            match fs::read_to_string(path) {
                                Ok(source) => {
                                    Self::get_intermediate_contract(&source, res_map);
                                    None
                                }
                                Err(_) => None,
                            }
                        }
                    },
                    pt::SourceUnitPart::ContractDefinition(cd) => {
                        let mut intermediate = IntermediateContract::default();

                        cd.parts.into_iter().for_each(|part| match part {
                            pt::ContractPart::FunctionDefinition(def) => {
                                // Only match normal function definitions here.
                                if matches!(def.ty, pt::FunctionTy::Function) {
                                    intermediate
                                        .function_definitions
                                        .insert(def.name.clone().unwrap().name, def);
                                }
                            }
                            pt::ContractPart::EventDefinition(def) => {
                                let event_name = def.name.safe_unwrap().name.clone();
                                intermediate.event_definitions.insert(event_name, def);
                            }
                            pt::ContractPart::StructDefinition(def) => {
                                let struct_name = def.name.safe_unwrap().name.clone();
                                intermediate.struct_definitions.insert(struct_name, def);
                            }
                            pt::ContractPart::VariableDefinition(def) => {
                                let var_name = def.name.safe_unwrap().name.clone();
                                intermediate.variable_definitions.insert(var_name, def);
                            }
                            _ => {}
                        });
                        Some((cd.name.safe_unwrap().name.clone(), intermediate))
                    }
                    _ => None,
                })
                .collect::<HashMap<String, IntermediateContract>>();
            res_map.extend(func_defs);
        }
    }

    /// Helper to deconstruct a statement
    ///
    /// ### Takes
    ///
    /// A reference to a [pt::Statement]
    ///
    /// ### Returns
    ///
    /// A vector containing tuples of the inner expressions' names, types, and storage locations.
    pub fn get_statement_definitions(statement: &pt::Statement) -> Vec<(String, pt::Expression)> {
        match statement {
            pt::Statement::VariableDefinition(_, def, _) => {
                vec![(def.name.safe_unwrap().name.clone(), def.ty.clone())]
            }
            pt::Statement::Expression(_, pt::Expression::Assign(_, left, _)) => {
                if let pt::Expression::List(_, list) = left.as_ref() {
                    list.iter()
                        .filter_map(|(_, param)| {
                            param.as_ref().and_then(|param| {
                                param
                                    .name
                                    .as_ref()
                                    .map(|name| (name.name.clone(), param.ty.clone()))
                            })
                        })
                        .collect()
                } else {
                    Vec::default()
                }
            }
            _ => Vec::default(),
        }
    }
}

impl IntermediateOutput {
    /// Helper function that returns the body of the REPL contract's "run" function.
    ///
    /// ### Returns
    ///
    /// Optionally, the last statement within the "run" function of the REPL contract.
    pub fn run_func_body(&self) -> Result<&Vec<pt::Statement>> {
        match self
            .intermediate_contracts
            .get("REPL")
            .ok_or_else(|| eyre::eyre!("Could not find REPL intermediate contract!"))?
            .function_definitions
            .get("run")
            .ok_or_else(|| eyre::eyre!("Could not find run function definition in REPL contract!"))?
            .body
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Could not find run function body!"))?
        {
            pt::Statement::Block { statements, .. } => Ok(statements),
            _ => eyre::bail!("Could not find statements within run function body!"),
        }
    }
}

/// A Parse Tree Fragment
///
/// Used to determine whether an input will go to the "run()" function,
/// the top level of the contract, or in global scope.
#[derive(Debug)]
pub enum ParseTreeFragment {
    /// Code for the global scope
    Source,
    /// Code for the top level of the contract
    Contract,
    /// Code for the "run()" function
    Function,
}

/// Parses a fragment of solidity code with solang_parser and assigns
/// it a scope within the [SessionSource].
pub fn parse_fragment(
    solc: Solc,
    config: SessionSourceConfig,
    buffer: &str,
) -> Option<ParseTreeFragment> {
    let mut base = SessionSource::new(solc, config);

    match base.clone().with_run_code(buffer).parse() {
        Ok(_) => return Some(ParseTreeFragment::Function),
        Err(e) => debug_errors(&e),
    }
    match base.clone().with_top_level_code(buffer).parse() {
        Ok(_) => return Some(ParseTreeFragment::Contract),
        Err(e) => debug_errors(&e),
    }
    match base.with_global_code(buffer).parse() {
        Ok(_) => return Some(ParseTreeFragment::Source),
        Err(e) => debug_errors(&e),
    }

    None
}

fn debug_errors(errors: &[Diagnostic]) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }

    for error in errors {
        tracing::debug!("error: {}", error.message);
    }
}
