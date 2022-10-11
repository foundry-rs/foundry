//! A Session Source
//!
//! This module contains the `Source` struct, which is a concrete source constructed from solang_parser SolUnit parsed inputs.

use std::{collections::HashMap, path::PathBuf};

use eyre::Result;
use ethers_solc::{
    artifacts::{Contract, Source, Sources},
    CompilerInput, Solc,
};
use serde::{Serialize, Deserialize};

/// A compiled contract
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledContract {
    /// The compiled contract
    pub contract: Contract,
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
    pub variable_definitions: HashMap<String, (solang_parser::pt::Expression, Option<solang_parser::pt::StorageLocation>)>,
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
    /// Typically, global-level code is present between the contract definition and the first function (usually constructor)
    pub global_code: String,
    /// Top level solidity code
    /// Typically, this is code seen above the contructor
    pub top_level_code: String,
    /// Constructor Code
    pub constructor_code: String,
    /// Compiled Contracts
    pub compiled: Vec<CompiledContract>,
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
            compiled: vec![],
        }
    }

    // Fillers

    /// Appends global-level code to the source
    pub fn with_global_code(&mut self, content: &str) -> &mut Self {
        self.global_code.push_str(content);
        self.compiled = vec![];
        self
    }

    /// Appends top-level code to the source
    pub fn with_top_level_code(&mut self, content: &str) -> &mut Self {
        self.top_level_code.push_str(content);
        self.compiled = vec![];
        self
    }

    /// Appends constructor code to the source
    pub fn with_constructor_code(&mut self, content: &str) -> &mut Self {
        self.constructor_code.push_str(content);
        self.compiled = vec![];
        self
    }

    // Drains

    /// Clears global code from the source
    pub fn drain_global_code(&mut self) -> &mut Self {
        self.global_code = Default::default();
        self.compiled = vec![];
        self
    }

    /// Clears top-level code from the source
    pub fn drain_top_level_code(&mut self) -> &mut Self {
        self.top_level_code = Default::default();
        self.compiled = vec![];
        self
    }

    /// Clears the constructor code
    pub fn drain_constructor(&mut self) -> &mut Self {
        self.constructor_code = Default::default();
        self.compiled = vec![];
        self
    }

    /// Generates and ethers_solc::CompilerInput from the source
    pub fn compiler_input(&self) -> CompilerInput {
        let mut sources = Sources::new();
        sources.insert(
            self.file_name.clone(),
            Source {
                content: self.to_string(),
            },
        );
        CompilerInput::with_sources(sources).pop().unwrap()
    }

    /// Compiles the source using [solang_parser]()
    pub fn parse(&self) -> Result<solang_parser::pt::SourceUnit, Vec<solang_parser::diagnostics::Diagnostic>> {
        solang_parser::parse(&self.to_string(), 0).map(|(pt, _)| pt)
    }

    /// Decompose the solang_parser::pt::SourceUnit into parts
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
            return Err(eyre::eyre!("Missing pragma directive"));
        }
        source_unit_parts.remove(0);

        // Extract contract definitions
        let mut contract_parts = match source_unit_parts.pop().ok_or(eyre::eyre!("Failed to pop source unit part"))? {
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
        let statements = match contract_parts.pop().ok_or(eyre::eyre!("Failed to pop source unit part"))? {
            solang_parser::pt::ContractPart::FunctionDefinition(func) => {
                if !matches!(func.ty, solang_parser::pt::FunctionTy::Constructor) {
                    return Err(eyre::eyre!("Missing constructor"));
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

}

impl std::fmt::Display for SessionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("// SPDX-License-Identifier: UNLICENSED\n")?;
        let semver::Version {
            major,
            minor,
            patch,
            ..
        } = self.solc.version().unwrap();
        f.write_fmt(format_args!("pragma solidity ^{major}.{minor}.{patch};\n",))?;
        f.write_str(&self.global_code)?;
        f.write_fmt(format_args!("contract {} {{\n", self.contract_name))?;
        f.write_str(&self.top_level_code)?;
        f.write_str("constructor() {\n")?;
        f.write_str(&self.constructor_code)?;
        f.write_str("}\n}")?;
        Ok(())
    }
}