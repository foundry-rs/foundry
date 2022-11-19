//! Session Source
//!
//! This module contains the `SessionSource` struct, which is a minimal wrapper around
//! the REPL contract's source code. It provides simple compilation, parsing, and
//! execution helpers.

use ethers_solc::{
    artifacts::{Source, Sources},
    CompilerInput, CompilerOutput, Solc,
};
use eyre::Result;
use forge::executor::{opts::EvmOpts, Backend};
use foundry_config::Config;
use semver::Version;
use serde::{Deserialize, Serialize};
use solang_parser::pt::{self, CodeLocation};
use std::{collections::HashMap, path::PathBuf};

/// Solidity source for the `Vm` interface in [forge-std](https://github.com/foundry-rs/forge-std)
static VM_SOURCE: &'static str = include_str!("../../testdata/lib/forge-std/src/Vm.sol");

/// Intermediate output for the compiled [SessionSource]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntermediateOutput {
    /// The source unit parts
    #[serde(skip)]
    pub source_unit_parts: Vec<pt::SourceUnitPart>,
    /// Contract parts
    #[serde(skip)]
    pub contract_parts: Vec<pt::ContractPart>,
    /// Contract statements
    #[serde(skip)]
    pub statements: Vec<pt::Statement>,
    /// Contract variable definitions
    #[serde(skip)]
    pub variable_definitions: HashMap<String, (pt::Expression, Option<pt::StorageLocation>)>,
    /// Intermediate contracts
    pub intermediate_contracts: HashMap<String, IntermediateContract>,
}

/// A refined intermediate parse tree for a contract that enables easy lookups
/// of definitions.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// All variable definitions within the contract
    #[serde(skip)]
    pub variable_definitions: HashMap<String, Box<pt::VariableDefinition>>,
}

/// Full compilation output for the [SessionSource]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedOutput {
    /// The [IntermediateOutput] component
    pub intermediate: IntermediateOutput,
    /// The [CompilerOutput] component
    pub compiler_output: CompilerOutput,
}

/// Configuration for the [SessionSource]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSourceConfig {
    /// Foundry configuration
    pub foundry_config: Config,
    /// EVM Options
    pub evm_opts: EvmOpts,
    #[serde(skip)]
    /// In-memory REVM db for the session's runner.
    pub backend: Option<Backend>,
    /// Optionally enable traces for the REPL contract execution
    pub traces: bool,
}

/// REPL Session Source wrapper
///
/// Heavily based on soli's [`ConstructedSource`](https://github.com/jpopesculian/soli/blob/master/src/main.rs#L166)
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Typically, this is code seen above the contructor
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
    /// ### Takes
    ///
    /// - A reference to a [Solc] instance
    /// - A reference to a [SessionSourceConfig]
    ///
    /// ### Returns
    ///
    /// A blank [SessionSource]
    pub fn new(solc: &Solc, config: &SessionSourceConfig) -> Self {
        assert!(solc.version().is_ok());
        Self {
            file_name: PathBuf::from("ReplContract.sol".to_string()),
            contract_name: "REPL".to_string(),
            solc: solc.clone(),
            global_code: Default::default(),
            top_level_code: Default::default(),
            run_code: Default::default(),
            generated_output: None,
            config: config.clone(),
        }
    }

    // Clones a [SessionSource] without copying the [GeneratedOutput], as it will
    // need to be regenerated as soon as new code is added.
    //
    // ### Returns
    //
    // A shallow-cloned [SessionSource]
    fn shallow_clone(&self) -> Self {
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
    pub fn clone_with_new_line(&self, mut content: String) -> Result<(SessionSource, bool)> {
        let mut new_source = self.shallow_clone();
        if let Some(parsed) = parse_fragment(&new_source.solc, &new_source.config, &content)
            .or_else(|| {
                content = format!("{};", content);
                parse_fragment(&new_source.solc, &new_source.config, &content)
            })
            .or_else(|| {
                parse_fragment(
                    &new_source.solc,
                    &new_source.config,
                    &content.trim_end().trim_end_matches(';').to_string(),
                )
            })
        {
            // Flag that tells the dispatcher whether to build or execute the session
            // source based on the scope of the new code.
            match parsed {
                ParseTreeFragment::Function(_) => new_source.with_run_code(&content),
                ParseTreeFragment::Contract(_) => new_source.with_top_level_code(&content),
                ParseTreeFragment::Source(_) => new_source.with_global_code(&content),
            };

            Ok((new_source, matches!(parsed, ParseTreeFragment::Function(_))))
        } else {
            eyre::bail!("\"{}\"", content.trim().to_owned());
        }
    }

    // Fillers

    /// Appends global-level code to the source
    pub fn with_global_code(&mut self, content: &str) -> &mut Self {
        self.global_code.push_str(&format!("{}\n", content.trim()));
        self.generated_output = None;
        self
    }

    /// Appends top-level code to the source
    pub fn with_top_level_code(&mut self, content: &str) -> &mut Self {
        self.top_level_code.push_str(&format!("{}\n", content.trim()));
        self.generated_output = None;
        self
    }

    /// Appends code to the "run()" function
    pub fn with_run_code(&mut self, content: &str) -> &mut Self {
        self.run_code.push_str(&format!("{}\n", content.trim()));
        self.generated_output = None;
        self
    }

    // Drains

    /// Clears global code from the source
    pub fn drain_global_code(&mut self) -> &mut Self {
        self.global_code = Default::default();
        self.generated_output = None;
        self
    }

    /// Clears top-level code from the source
    pub fn drain_top_level_code(&mut self) -> &mut Self {
        self.top_level_code = Default::default();
        self.generated_output = None;
        self
    }

    /// Clears the "run()" function's code
    pub fn drain_run(&mut self) -> &mut Self {
        self.run_code = Default::default();
        self.generated_output = None;
        self
    }

    /// Generates and ethers_solc::CompilerInput from the source
    ///
    /// ### Returns
    ///
    /// A [CompilerInput] object containing forge-std's `Vm` interface as well as the REPL contract
    /// source.
    pub fn compiler_input(&self) -> CompilerInput {
        let mut sources = Sources::new();
        sources.insert(PathBuf::from("forge-std/Vm.sol"), Source { content: VM_SOURCE.to_owned() });
        sources.insert(self.file_name.clone(), Source { content: self.to_repl_source() });
        CompilerInput::with_sources(sources).pop().unwrap()
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

    /// Decompose the parsed [pt::SourceUnit] into parts
    ///
    /// ### Takes
    ///
    /// A [pt::SourceUnit] representing a parsed Solidity file.
    ///
    /// ### Returns
    ///
    /// Optionally, SourceUnitParts, ContractParts, and Statements
    pub fn decompose(
        &self,
        source_unit: pt::SourceUnit,
    ) -> Result<(Vec<pt::SourceUnitPart>, Vec<pt::ContractPart>, Vec<pt::Statement>)> {
        // Extract the SourceUnitParts from the source_unit
        let pt::SourceUnit(mut source_unit_parts) = source_unit;

        // The first item in the source unit should be the pragma directive
        if !matches!(source_unit_parts.get(0), Some(pt::SourceUnitPart::PragmaDirective(..))) {
            eyre::bail!("Missing pragma directive");
        }
        source_unit_parts.remove(0);

        // Extract contract definitions
        let mut contract_parts =
            match source_unit_parts.pop().ok_or(eyre::eyre!("Failed to pop source unit part"))? {
                pt::SourceUnitPart::ContractDefinition(contract) => {
                    if contract.name.name == self.contract_name {
                        Ok(contract.parts)
                    } else {
                        Err(eyre::eyre!("Contract name mismatch"))
                    }
                }
                _ => Err(eyre::eyre!("Missing contract definition")),
            }?;

        // Parse Statements
        let statements =
            match contract_parts.pop().ok_or(eyre::eyre!("Failed to pop source unit part"))? {
                pt::ContractPart::FunctionDefinition(func) => {
                    if !matches!(func.ty, pt::FunctionTy::Function) {
                        eyre::bail!("Missing run() function");
                    }
                    match func.body.ok_or(eyre::eyre!("Missing run() Function Body"))? {
                        pt::Statement::Block { statements, .. } => Ok(statements),
                        _ => Err(eyre::eyre!("Invalid run() function body")),
                    }
                }
                _ => Err(eyre::eyre!("Contract missing function definition")),
            }?;

        // Return the parts
        Ok((source_unit_parts, contract_parts, statements))
    }

    /// Parses and decomposes the source
    ///
    /// ### Returns
    ///
    /// Optionally, a tuple containing a vec of [pt::SourceUnitPart]s, a vec of [pt::ContractPart]s,
    /// and a vec of [pt::Statement]s
    pub fn parse_and_decompose(
        &self,
    ) -> Result<(Vec<pt::SourceUnitPart>, Vec<pt::ContractPart>, Vec<pt::Statement>)> {
        let parse_tree =
            self.parse().map_err(|_| eyre::eyre!("Failed to generate SourceUnit from Source"))?;
        self.decompose(parse_tree)
    }

    /// Generate intermediate contracts for all contract definitions in the compilation source.
    ///
    /// TODO: Clean - we don't need to re-parse the REPL source. Should pass this in.
    ///
    /// ### Returns
    ///
    /// Optionally, a map of contract names to a vec of [IntermediateContract]s.
    pub fn generate_intermediate_contracts(&self) -> Result<HashMap<String, IntermediateContract>> {
        let mut res_map = HashMap::new();
        let parsed_map = self.compiler_input().sources;
        for source in parsed_map.values() {
            if let Ok((pt::SourceUnit(source_unit_parts), _)) =
                solang_parser::parse(&source.content, 0)
            {
                let func_defs = source_unit_parts
                    .into_iter()
                    .filter_map(|sup| match sup {
                        pt::SourceUnitPart::ContractDefinition(cd) => {
                            let mut intermediate = IntermediateContract::default();

                            cd.parts.into_iter().for_each(|part| match part {
                                pt::ContractPart::FunctionDefinition(def) => {
                                    if matches!(def.ty, pt::FunctionTy::Function) {
                                        intermediate
                                            .function_definitions
                                            .insert(def.name.clone().unwrap().name, def);
                                    }
                                }
                                pt::ContractPart::EventDefinition(def) => {
                                    intermediate
                                        .event_definitions
                                        .insert(def.name.name.clone(), def);
                                }
                                pt::ContractPart::StructDefinition(def) => {
                                    intermediate
                                        .struct_definitions
                                        .insert(def.name.name.clone(), def);
                                }
                                pt::ContractPart::VariableDefinition(def) => {
                                    intermediate
                                        .variable_definitions
                                        .insert(def.name.name.clone(), def);
                                }
                                _ => {}
                            });
                            Some((cd.name.name, intermediate))
                        }
                        _ => None,
                    })
                    .collect::<HashMap<String, IntermediateContract>>();
                res_map.extend(func_defs);
            }
        }
        Ok(res_map)
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

        // Parse and decompose into parts
        let (source_unit_parts, contract_parts, statements) = self.parse_and_decompose()?;

        // Construct variable definitions
        let mut variable_definitions = HashMap::new();
        for (key, ty) in contract_parts.iter().flat_map(Self::get_contract_part_definition) {
            variable_definitions
                .insert(key.to_string(), (ty.clone(), Some(pt::StorageLocation::Memory(ty.loc()))));
        }
        for (key, ty, storage) in statements.iter().flat_map(Self::get_statement_definitions) {
            variable_definitions.insert(key.to_string(), (ty.clone(), storage.cloned()));
        }

        // Construct intermediate output
        let intermediate_output = IntermediateOutput {
            source_unit_parts,
            contract_parts,
            statements,
            variable_definitions,
            intermediate_contracts: self.generate_intermediate_contracts()?,
        };

        // Construct a Compiled Result
        let generated_output =
            GeneratedOutput { intermediate: intermediate_output, compiler_output };
        self.generated_output = Some(generated_output.clone()); // ehhh, need to not clone this.
        Ok(generated_output)
    }

    /// Helper to convert a ContractPart into a VariableDefinition
    ///
    /// ### Takes
    ///
    /// A reference to a [pt::ContractPart]
    ///
    /// ### Returns
    ///
    /// Optionally, a tuple containing the [pt::ContractPart::VariableDefinition]'s name and type.
    pub fn get_contract_part_definition(
        contract_part: &pt::ContractPart,
    ) -> Option<(&str, &pt::Expression)> {
        match contract_part {
            pt::ContractPart::VariableDefinition(var_def) => {
                Some((&var_def.name.name, &var_def.ty))
            }
            _ => None,
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
    pub fn get_statement_definitions(
        statement: &pt::Statement,
    ) -> Vec<(&str, &pt::Expression, Option<&pt::StorageLocation>)> {
        match statement {
            pt::Statement::VariableDefinition(_, def, _) => {
                vec![(def.name.name.as_str(), &def.ty, def.storage.as_ref())]
            }
            pt::Statement::Expression(_, pt::Expression::Assign(_, left, _)) => {
                if let pt::Expression::List(_, list) = left.as_ref() {
                    list.iter()
                        .filter_map(|(_, param)| {
                            param.as_ref().and_then(|param| {
                                param
                                    .name
                                    .as_ref()
                                    .map(|name| (name.name.as_str(), &param.ty, None))
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

    /// Convert the [SessionSource] to a valid Script contract
    ///
    /// ### Returns
    ///
    /// The [SessionSource] represented as a Forge Script contract.
    pub fn to_script_source(&self) -> String {
        let Version { major, minor, patch, .. } = self.solc.version().unwrap();
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^{major}.{minor}.{patch};

import {{Script}} from "forge-std/Script.sol";
{}

contract {} is Script {{
    {}
    
    /// @notice Script entry point
    function run() public {{
        {}
    }}
}}
            "#,
            self.global_code, self.contract_name, self.top_level_code, self.run_code,
        )
    }

    /// Convert the [SessionSource] to a valid REPL contract
    ///
    /// ### Returns
    ///
    /// The [SessionSource] represented as a REPL contract.
    pub fn to_repl_source(&self) -> String {
        let Version { major, minor, patch, .. } = self.solc.version().unwrap();
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^{major}.{minor}.{patch};

import {{Vm}} from "forge-std/Vm.sol";
{}

contract {} {{
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    {}
  
    /// @notice REPL contract entry point
    function run() public {{
        {}
    }}
}}
            "#,
            self.global_code, self.contract_name, self.top_level_code, self.run_code,
        )
    }
}

/// A Parse Tree Fragment
///
/// Used to determine whether an input will go to the "run()" function,
/// the top level of the contract, or in global scope.
#[derive(Debug)]
pub enum ParseTreeFragment {
    /// Code for the global scope
    Source(Vec<pt::SourceUnitPart>),
    /// Code for the top level of the contract
    Contract(Vec<pt::ContractPart>),
    /// Code for the "run()" function
    Function(Vec<pt::Statement>),
}

/// Parses a fragment of solidity code with solang_parser and assigns
/// it a scope within the [SessionSource].
pub fn parse_fragment(
    solc: &Solc,
    config: &SessionSourceConfig,
    buffer: &str,
) -> Option<ParseTreeFragment> {
    let base = SessionSource::new(solc, config);

    if let Ok((_, _, statements)) = base.clone().with_run_code(buffer).parse_and_decompose() {
        return Some(ParseTreeFragment::Function(statements))
    }
    if let Ok((_, contract_parts, _)) =
        base.clone().with_top_level_code(buffer).parse_and_decompose()
    {
        return Some(ParseTreeFragment::Contract(contract_parts))
    }
    if let Ok((source_unit_parts, _, _)) =
        base.clone().with_global_code(buffer).parse_and_decompose()
    {
        return Some(ParseTreeFragment::Source(source_unit_parts))
    }

    None
}
