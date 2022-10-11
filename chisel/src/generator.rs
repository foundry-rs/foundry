//! A Session Source
//!
//! This module contains the `Source` struct, which is a concrete source constructed from
//! solang_parser SolUnit parsed inputs.

use std::{collections::HashMap, path::PathBuf};

use ethers_solc::{
    artifacts::{Source, Sources},
    CompilerInput, CompilerOutput, Solc,
};
use eyre::Result;
use serde::{Deserialize, Serialize};
use solang_parser::pt::CodeLocation;

/// A compiled contract
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

/// GeneratedOutput
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedOutput {
    /// The intermediate output
    pub intermediate: IntermediateOutput,
    /// The CompilerOutput
    pub compiler_output: CompilerOutput,
}

/// A Session Source
///
/// Heavily based on soli's [`ConstructedSource`](https://github.com/jpopesculian/soli/blob/master/src/main.rs#L166)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSource {
    /// The file name
    pub file_name: PathBuf,
    /// The contract name
    pub contract_name: String,
    /// The solidity compiler version
    pub solc: Solc,
    /// Global level solidity code
    /// Typically, global-level code is present between the contract definition and the first
    /// function (usually constructor)
    pub global_code: String,
    /// Top level solidity code
    /// Typically, this is code seen above the contructor
    pub top_level_code: String,
    /// Constructor Code
    pub constructor_code: String,
    /// The solc compiler output
    pub compiled: Option<CompilerOutput>,
    /// The intermediate output
    pub intermediate: Option<IntermediateOutput>,
    /// The generated output
    pub generated_output: Option<GeneratedOutput>,
}

impl SessionSource {
    /// Creates a new source given a solidity compiler version
    pub fn new(solc: &Solc) -> Self {
        assert!(solc.version().is_ok());
        Self {
            file_name: PathBuf::from("ReplContract.sol".to_string()),
            contract_name: "REPL".to_string(),
            solc: solc.clone(),
            global_code: Default::default(),
            top_level_code: Default::default(),
            constructor_code: Default::default(),
            compiled: None,
            intermediate: None,
            generated_output: None,
        }
    }

    // Fillers

    /// Appends global-level code to the source
    pub fn with_global_code(&mut self, content: &str) -> &mut Self {
        self.global_code.push_str(content);
        self.compiled = None;
        self.intermediate = None;
        self.generated_output = None;
        self
    }

    /// Appends top-level code to the source
    pub fn with_top_level_code(&mut self, content: &str) -> &mut Self {
        self.top_level_code.push_str(content);
        self.compiled = None;
        self.intermediate = None;
        self.generated_output = None;
        self
    }

    /// Appends constructor code to the source
    pub fn with_constructor_code(&mut self, content: &str) -> &mut Self {
        self.constructor_code.push_str(content);
        self.compiled = None;
        self.intermediate = None;
        self.generated_output = None;
        self
    }

    // Drains

    /// Clears global code from the source
    pub fn drain_global_code(&mut self) -> &mut Self {
        self.global_code = Default::default();
        self.compiled = None;
        self.intermediate = None;
        self.generated_output = None;
        self
    }

    /// Clears top-level code from the source
    pub fn drain_top_level_code(&mut self) -> &mut Self {
        self.top_level_code = Default::default();
        self.compiled = None;
        self.intermediate = None;
        self.generated_output = None;
        self
    }

    /// Clears the constructor code
    pub fn drain_constructor(&mut self) -> &mut Self {
        self.constructor_code = Default::default();
        self.compiled = None;
        self.intermediate = None;
        self.generated_output = None;
        self
    }

    /// Generates and ethers_solc::CompilerInput from the source
    pub fn compiler_input(&self) -> CompilerInput {
        let mut sources = Sources::new();
        sources.insert(self.file_name.clone(), Source { content: self.to_string() });
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
            return Err(eyre::eyre!("Missing pragma directive"))
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
                    if !matches!(func.ty, solang_parser::pt::FunctionTy::Constructor) {
                        return Err(eyre::eyre!("Missing constructor"))
                    }
                    match func.body.ok_or(eyre::eyre!("Missing Constructor Function Body"))? {
                        solang_parser::pt::Statement::Block { statements, .. } => Ok(statements),
                        _ => Err(eyre::eyre!("Invalid constructor function body")),
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
            return Err(eyre::eyre!("Compiler errors: {:?}", errors))
        }

        Ok(compiled)

        // Get all compiled contracts for our file name
        // let mut contracts_for_file = compiled
        //     .contracts
        //     .remove(&self.file_name.display().to_string()).ok_or(eyre::eyre!("Failed to find
        // compiled sources for file name"))?;

        // // Extract the matching contract
        // contracts_for_file.remove(&self.contract_name).ok_or(eyre::eyre!("Missing compiled source
        // for contract name"))
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
        f.write_fmt(format_args!("pragma solidity ^{major}.{minor}.{patch};\n",))?;
        f.write_str("\n")?;

        // Global imports and definitions
        f.write_str(&self.global_code)?;

        f.write_fmt(format_args!("contract {} {{\n", self.contract_name))?;
        f.write_str(&self.top_level_code)?;
        f.write_str("constructor() {\n")?;
        f.write_str(&self.constructor_code)?;
        f.write_str("}\n}")?;
        Ok(())
    }
}
