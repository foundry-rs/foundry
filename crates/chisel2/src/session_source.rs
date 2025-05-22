//! Session Source
//!
//! This module contains the `SessionSource` struct, which is a minimal wrapper around
//! the REPL contract's source code. It provides simple compilation, parsing, and
//! execution helpers.

use alloy_primitives::map::HashMap;
use eyre::Result;
use foundry_compilers::{
    artifacts::{CompilerOutput, Settings, SolcInput, Source, Sources},
    compilers::solc::Solc,
};
use foundry_config::{Config, SolcReq};
use foundry_evm::{backend::Backend, opts::EvmOpts};
use semver::Version;
use serde::{Deserialize, Serialize};
use solang_parser::pt;
use solar_parse::interface::diagnostics::EmittedDiagnostics;
use std::path::PathBuf;
use walkdir::WalkDir;

/// The minimum Solidity version of the `Vm` interface.
pub const MIN_VM_VERSION: Version = Version::new(0, 6, 2);

/// Solidity source for the `Vm` interface in [forge-std](https://github.com/foundry-rs/forge-std)
static VM_SOURCE: &str = include_str!("../../../testdata/cheats/Vm.sol");

/// Intermediate output for the compiled [SessionSource]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntermediateOutput {
    /// All expressions within the REPL contract's run function and top level scope.
    pub repl_contract_expressions: HashMap<String, pt::Expression>,
    /// Intermediate contracts
    pub intermediate_contracts: IntermediateContracts,
}

/// A refined intermediate parse tree for a contract that enables easy lookups
/// of definitions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntermediateContract {
    /// All function definitions within the contract
    pub function_definitions: HashMap<String, Box<pt::FunctionDefinition>>,
    /// All event definitions within the contract
    pub event_definitions: HashMap<String, Box<pt::EventDefinition>>,
    /// All struct definitions within the contract
    pub struct_definitions: HashMap<String, Box<pt::StructDefinition>>,
    /// All variable definitions within the top level scope of the contract
    pub variable_definitions: HashMap<String, Box<pt::VariableDefinition>>,
}

/// A defined type for a map of contract names to [IntermediateContract]s
type IntermediateContracts = HashMap<String, IntermediateContract>;

/// Full compilation output for the [SessionSource]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedOutput {
    /// The [IntermediateOutput] component
    #[serde(skip)]
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
    /// In-memory REVM db for the session's runner.
    #[serde(skip)]
    pub backend: Option<Backend>,
    /// Optionally enable traces for the REPL contract execution
    pub traces: bool,
    /// Optionally set calldata for the REPL contract execution
    pub calldata: Option<Vec<u8>>,
}

impl SessionSourceConfig {
    /// Returns the solc version to use as defined in the config, or the default (0.8.19).
    pub(crate) fn solc(&mut self) -> Result<Solc> {
        if self.foundry_config.solc.is_none() {
            self.foundry_config.solc = Some(SolcReq::Version(Version::new(0, 8, 19)));
        }
        match self.foundry_config.solc_compiler()? {
            foundry_compilers::solc::SolcCompiler::AutoDetect => unreachable!(),
            foundry_compilers::solc::SolcCompiler::Specific(solc) => Ok(solc),
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
            evm_version: self
                .config
                .foundry_config
                .evm_version
                .normalize_version_solc(&self.solc.version),
            ..Default::default()
        };

        // we only care about the solidity source, so we can safely unwrap
        SolcInput::resolve_and_build(sources, settings)
            .into_iter()
            .next()
            .map(|i| i.sanitized(&self.solc.version))
            .expect("Solidity source not found")
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
    pub fn build(&mut self) -> Result<&GeneratedOutput> {
        let compiler_output = self.compile()?;
        let intermediate_output = self.analyze()?;
        let generated_output =
            GeneratedOutput { intermediate: intermediate_output, compiler_output };
        Ok(self.generated_output.insert(generated_output))
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
        let (mut vm_import, mut vm_constant) = (String::new(), String::new());
        if !config.no_vm {
            // Check if there's any `forge-std` remapping and determine proper path to it by
            // searching remapping path.
            if let Some(remapping) = config
                .foundry_config
                .remappings
                .iter()
                .find(|remapping| remapping.name == "forge-std/")
            {
                if let Some(vm_path) = WalkDir::new(&remapping.path.path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .find(|e| e.file_name() == "Vm.sol")
                {
                    vm_import = format!("import {{Vm}} from \"{}\";\n", vm_path.path().display());
                    vm_constant = "Vm internal constant vm = Vm(address(uint160(uint256(keccak256(\"hevm cheat code\")))));\n".to_string();
                }
            }
        }

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

    pub fn parse(&self) -> Result<(), EmittedDiagnostics> {
        let sess = self.make_session();
        let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
            let arena = solar_parse::ast::Arena::new();
            let filename = self.file_name.clone().into();
            let src = self.to_repl_source();
            let mut parser = solar_parse::Parser::from_source_code(&sess, &arena, filename, src)?;
            let _ast = parser.parse_file().map_err(|e| e.emit())?;
            Ok(())
        });
        sess.dcx.emitted_errors().unwrap()
    }

    pub fn analyze(&self) -> Result<IntermediateOutput, EmittedDiagnostics> {
        todo!()
    }

    fn make_session(&self) -> solar_parse::interface::Session {
        // TODO(dani): use future common utilities for solc input -> solar session
        solar_parse::interface::Session::builder().with_buffer_emitter(Default::default()).build()
    }
}

impl IntermediateOutput {
    /// Helper function that returns the body of the REPL contract's "run" function.
    ///
    /// ### Returns
    ///
    /// Optionally, the last statement within the "run" function of the REPL contract.
    pub fn run_func_body(&self) -> Result<&[pt::Statement]> {
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
enum ParseTreeFragment {
    /// Code for the global scope
    Source,
    /// Code for the top level of the contract
    Contract,
    /// Code for the "run()" function
    Function,
}

/// Parses a fragment of solidity code with solang_parser and assigns
/// it a scope within the [SessionSource].
fn parse_fragment(
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

#[track_caller]
fn debug_errors(errors: &EmittedDiagnostics) {
    tracing::debug!("{errors}");
}
