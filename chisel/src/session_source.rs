//! Session Source
//!
//! This module contains the `SessionSource` struct, which is a minimal wrapper around
//! the REPL contract's source code. It provides simple compilation, parsing, and
//! execution helpers.

use crate::SCRIPT_PATH;
use ethers_solc::{
    artifacts::{Source, Sources},
    CompilerInput, CompilerOutput, Solc,
};
use eyre::Result;
use forge::executor::{opts::EvmOpts, Backend};
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use solang_parser::pt::CodeLocation;
use std::{collections::HashMap, fs, path::PathBuf};

/// Intermediate output for the compiled [SessionSource]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntermediateOutput {
    /// The source unit parts
    #[serde(skip)]
    pub source_unit_parts: Vec<solang_parser::pt::SourceUnitPart>,
    /// Contract parts
    #[serde(skip)]
    pub contract_parts: Vec<solang_parser::pt::ContractPart>,
    /// Contract statements
    #[serde(skip)]
    pub statements: Vec<solang_parser::pt::Statement>,
    /// Contract variable definitions
    #[serde(skip)]
    pub variable_definitions: HashMap<
        String,
        (solang_parser::pt::Expression, Option<solang_parser::pt::StorageLocation>),
    >,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSourceConfig {
    /// Foundry configuration
    pub config: Config,
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

    /// Clones the [SessionSource] and appends a new line of code. Will return
    /// an error result if the new line fails to be parsed.
    pub fn clone_with_new_line(&self, mut content: String) -> Result<SessionSource> {
        let mut new_source = self.clone();
        if let Some(parsed) = parse_fragment(&new_source.solc, &self.config, &content)
            .or_else(|| {
                content = content.trim_end().to_string();
                content.push_str(";\n");
                parse_fragment(&new_source.solc, &self.config, &content)
            })
            .or_else(|| {
                content = content.trim_end().trim_end_matches(';').to_string();
                content.push('\n');
                parse_fragment(&new_source.solc, &self.config, &format!("\t{}", content))
            })
        {
            match parsed {
                ParseTreeFragment::Function(_) => new_source.with_run_code(&content),
                ParseTreeFragment::Contract(_) => new_source.with_top_level_code(&content),
                ParseTreeFragment::Source(_) => new_source.with_global_code(&content),
            };

            Ok(new_source)
        } else {
            eyre::bail!(content.trim().to_owned());
        }
    }

    // Fillers

    /// Appends global-level code to the source
    pub fn with_global_code(&mut self, content: &str) -> &mut Self {
        self.global_code.push_str(format!("{}\n", content.trim()).as_str());
        self.generated_output = None;
        self
    }

    /// Appends top-level code to the source
    pub fn with_top_level_code(&mut self, content: &str) -> &mut Self {
        self.top_level_code.push_str(format!("\t{}\n", content.trim()).as_str());
        self.generated_output = None;
        self
    }

    /// Appends code to the "run()" function
    pub fn with_run_code(&mut self, content: &str) -> &mut Self {
        self.run_code.push_str(format!("\t\t{}\n", content.trim()).as_str());
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
    pub fn compiler_input(&self) -> CompilerInput {
        let mut sources = Sources::new();
        sources.insert(self.file_name.clone(), Source { content: self.to_string() });
        sources.insert(
            SCRIPT_PATH.clone(),
            Source { content: fs::read_to_string(SCRIPT_PATH.as_path()).unwrap() },
        );
        CompilerInput::with_sources(sources).pop().unwrap()
    }

    /// Compiles the source using [solang_parser]()
    pub fn parse(
        &self,
    ) -> Result<solang_parser::pt::SourceUnit, Vec<solang_parser::diagnostics::Diagnostic>> {
        solang_parser::parse(&self.to_string(), 0).map(|(pt, _)| pt)
    }

    /// Decompose the parsed solang_parser::pt::SourceUnit into parts
    ///
    /// ### Returns
    ///
    /// Optionally, SourceUnitParts, ContractParts, and Statements
    pub fn decompose(
        &self,
        source_unit: solang_parser::pt::SourceUnit,
    ) -> Result<(
        Vec<solang_parser::pt::SourceUnitPart>,
        Vec<solang_parser::pt::ContractPart>,
        Vec<solang_parser::pt::Statement>,
    )> {
        // Extract the SourceUnitParts from the source_unit
        let mut source_unit_parts = source_unit.0;

        // The first item in the source unit should be the pragma directive
        if !matches!(
            source_unit_parts.get(0),
            Some(solang_parser::pt::SourceUnitPart::PragmaDirective(..))
        ) {
            eyre::bail!("Missing pragma directive");
        }
        source_unit_parts.remove(0);

        // Extract contract definitions
        let mut contract_parts =
            match source_unit_parts.pop().ok_or(eyre::eyre!("Failed to pop source unit part"))? {
                solang_parser::pt::SourceUnitPart::ContractDefinition(contract) => {
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
                solang_parser::pt::ContractPart::FunctionDefinition(func) => {
                    if !matches!(func.ty, solang_parser::pt::FunctionTy::Function) {
                        eyre::bail!("Missing run() function");
                    }
                    match func.body.ok_or(eyre::eyre!("Missing run() Function Body"))? {
                        solang_parser::pt::Statement::Block { statements, .. } => Ok(statements),
                        _ => Err(eyre::eyre!("Invalid run() function body")),
                    }
                }
                _ => Err(eyre::eyre!("Contract missing function definition")),
            }?;

        // Return the parts
        Ok((source_unit_parts, contract_parts, statements))
    }

    /// Parses and decomposes the source
    pub fn parse_and_decompose(
        &self,
    ) -> Result<(
        Vec<solang_parser::pt::SourceUnitPart>,
        Vec<solang_parser::pt::ContractPart>,
        Vec<solang_parser::pt::Statement>,
    )> {
        let parse_tree =
            self.parse().map_err(|_| eyre::eyre!("Failed to generate SourceUnit from Source"))?;
        self.decompose(parse_tree)
    }

    /// Compile the contract
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
    pub fn build(&mut self) -> Result<GeneratedOutput> {
        // Use the cached compiled source if it exists
        if let Some(generated_output) = self.generated_output.as_ref() {
            return Ok(generated_output.clone())
        }

        // Compile
        let compiler_output = self.compile()?;

        // Parse and decompose into parts
        let (source_unit_parts, contract_parts, statements) = self.parse_and_decompose()?;

        // Construct variable definitions
        let mut variable_definitions = HashMap::new();
        for (key, ty) in contract_parts.iter().flat_map(Self::get_contract_part_definition) {
            variable_definitions.insert(
                key.to_string(),
                (ty.clone(), Some(solang_parser::pt::StorageLocation::Memory(ty.loc()))),
            );
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
        };

        // Construct a Compiled Result
        let generated_output =
            GeneratedOutput { intermediate: intermediate_output, compiler_output };
        self.generated_output = Some(generated_output.clone());
        Ok(generated_output)
    }

    /// Helper to convert a ContractPart into a VariableDefinition
    pub fn get_contract_part_definition(
        contract_part: &solang_parser::pt::ContractPart,
    ) -> Option<(&str, &solang_parser::pt::Expression)> {
        match contract_part {
            solang_parser::pt::ContractPart::VariableDefinition(var_def) => {
                Some((&var_def.name.name, &var_def.ty))
            }
            _ => None,
        }
    }

    /// Helper to deconstruct a statement
    pub fn get_statement_definitions(
        statement: &solang_parser::pt::Statement,
    ) -> Vec<(&str, &solang_parser::pt::Expression, Option<&solang_parser::pt::StorageLocation>)>
    {
        match statement {
            solang_parser::pt::Statement::VariableDefinition(_, def, _) => {
                vec![(def.name.name.as_str(), &def.ty, def.storage.as_ref())]
            }
            solang_parser::pt::Statement::Expression(
                _,
                solang_parser::pt::Expression::Assign(_, left, _),
            ) => {
                if let solang_parser::pt::Expression::List(_, list) = left.as_ref() {
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
                    vec![]
                }
            }
            _ => vec![],
        }
    }
}

impl std::fmt::Display for SessionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Write the license and solidity pragma version
        f.write_str("// SPDX-License-Identifier: UNLICENSED\n")?;
        let semver::Version { major, minor, patch, .. } = self.solc.version().unwrap();
        f.write_fmt(format_args!("pragma solidity ^{major}.{minor}.{patch};\n\n",))?;
        f.write_fmt(format_args!(
            "import {{Script}} from \"{}\";\n",
            SCRIPT_PATH.to_str().unwrap()
        ))?;

        // Global imports and definitions
        f.write_str(&self.global_code)?;
        f.write_str("\n")?;

        f.write_fmt(format_args!("contract {} is Script {{\n", self.contract_name))?;
        f.write_str(&self.top_level_code)?;
        f.write_str("\n")?;
        f.write_str("\tfunction run() external {\n")?;
        f.write_str(&self.run_code)?;
        f.write_str("\t}\n}")?;
        Ok(())
    }
}

/// A Parse Tree Fragment
///
/// Used to determine whether an input will go to the "run()" function,
/// the top level of the contract, or in global scope.
#[derive(Debug)]
pub enum ParseTreeFragment {
    /// Code for the global scope
    Source(Vec<solang_parser::pt::SourceUnitPart>),
    /// Code for the top level of the contract
    Contract(Vec<solang_parser::pt::ContractPart>),
    /// Code for the "run()" function
    Function(Vec<solang_parser::pt::Statement>),
}

/// Parses a fragment of solidity code with solang_parser and assigns
/// it a scope within the [SessionSource].
fn parse_fragment(
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
