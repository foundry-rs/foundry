//! A configurable artifacts handler implementation
//!
//! Configuring artifacts requires two pieces: the `ConfigurableArtifacts` handler, which contains
//! the configuration of how to construct the `ConfigurableArtifact` type based on a `Contract`. The
//! `ConfigurableArtifacts` populates a single `Artifact`, the `ConfigurableArtifact`, by default
//! with essential entries only, such as `abi`, `bytecode`,..., but may include additional values
//! based on its `ExtraOutputValues` that maps to various objects in the solc contract output, see
//! also: [`OutputSelection`](foundry_compilers_artifacts::output_selection::OutputSelection). In
//! addition to that some output values can also be emitted as standalone files.

use crate::{
    sources::VersionedSourceFile, Artifact, ArtifactFile, ArtifactOutput, SolcConfig, SolcError,
    SourceFile,
};
use alloy_json_abi::JsonAbi;
use alloy_primitives::hex;
use foundry_compilers_artifacts::{
    bytecode::{CompactBytecode, CompactDeployedBytecode},
    contract::Contract,
    output_selection::{
        BytecodeOutputSelection, ContractOutputSelection, DeployedBytecodeOutputSelection,
        EvmOutputSelection, EwasmOutputSelection,
    },
    BytecodeObject, ConfigurableContractArtifact, Evm, Ewasm, GeneratedSource, LosslessMetadata,
    Metadata, Settings,
};
use foundry_compilers_core::utils;
use std::{fs, path::Path};

/// An `Artifact` implementation that can be configured to include additional content and emit
/// additional files
///
/// Creates a single json artifact with
/// ```json
///  {
///    "abi": [],
///    "bytecode": {...},
///    "deployedBytecode": {...},
///    "methodIdentifiers": {...},
///    // additional values
///  }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigurableArtifacts {
    /// A set of additional values to include in the contract's artifact file
    pub additional_values: ExtraOutputValues,

    /// A set of values that should be written to a separate file
    pub additional_files: ExtraOutputFiles,

    /// PRIVATE: This structure may grow, As such, constructing this structure should
    /// _always_ be done using a public constructor or update syntax:
    ///
    /// ```
    /// use foundry_compilers::{ConfigurableArtifacts, ExtraOutputFiles};
    ///
    /// let config = ConfigurableArtifacts {
    ///     additional_files: ExtraOutputFiles { metadata: true, ..Default::default() },
    ///     ..Default::default()
    /// };
    /// ```
    #[doc(hidden)]
    pub __non_exhaustive: (),
}

impl ConfigurableArtifacts {
    pub fn new(
        extra_values: impl IntoIterator<Item = ContractOutputSelection>,
        extra_files: impl IntoIterator<Item = ContractOutputSelection>,
    ) -> Self {
        Self {
            additional_values: ExtraOutputValues::from_output_selection(extra_values),
            additional_files: ExtraOutputFiles::from_output_selection(extra_files),
            ..Default::default()
        }
    }

    /// Returns the `Settings` this configuration corresponds to
    pub fn solc_settings(&self) -> Settings {
        SolcConfig::builder()
            .additional_outputs(self.output_selection())
            .ast(self.additional_values.ast)
            .build()
    }

    /// Returns the output selection corresponding to this configuration
    pub fn output_selection(&self) -> Vec<ContractOutputSelection> {
        let mut selection = ContractOutputSelection::basic();

        let ExtraOutputValues {
            // handled above
            ast: _,
            userdoc,
            devdoc,
            method_identifiers,
            storage_layout,
            transient_storage_layout,
            assembly,
            legacy_assembly,
            gas_estimates,
            metadata,
            ir,
            ir_optimized,
            ir_optimized_ast,
            ewasm,
            function_debug_data,
            generated_sources,
            source_map,
            opcodes,
            __non_exhaustive,
        } = self.additional_values;

        if ir || self.additional_files.ir {
            selection.push(ContractOutputSelection::Ir);
        }
        if ir_optimized || self.additional_files.ir_optimized {
            selection.push(ContractOutputSelection::IrOptimized);
        }
        if metadata || self.additional_files.metadata {
            selection.push(ContractOutputSelection::Metadata);
        }
        if storage_layout {
            selection.push(ContractOutputSelection::StorageLayout);
        }
        if devdoc {
            selection.push(ContractOutputSelection::DevDoc);
        }
        if userdoc {
            selection.push(ContractOutputSelection::UserDoc);
        }
        if gas_estimates {
            selection.push(EvmOutputSelection::GasEstimates.into());
        }
        if assembly || self.additional_files.assembly {
            selection.push(EvmOutputSelection::Assembly.into());
        }
        if legacy_assembly || self.additional_files.legacy_assembly {
            selection.push(EvmOutputSelection::LegacyAssembly.into());
        }
        if ewasm || self.additional_files.ewasm {
            selection.push(EwasmOutputSelection::All.into());
        }
        if function_debug_data {
            selection.push(BytecodeOutputSelection::FunctionDebugData.into());
        }
        if method_identifiers {
            selection.push(EvmOutputSelection::MethodIdentifiers.into());
        }
        if generated_sources {
            selection.push(
                EvmOutputSelection::ByteCode(BytecodeOutputSelection::GeneratedSources).into(),
            );
        }
        if source_map {
            selection.push(EvmOutputSelection::ByteCode(BytecodeOutputSelection::SourceMap).into());
        }
        if ir_optimized_ast {
            selection.push(ContractOutputSelection::IrOptimizedAst);
        }
        if opcodes {
            selection.push(EvmOutputSelection::ByteCode(BytecodeOutputSelection::Opcodes).into());
        }
        if transient_storage_layout {
            selection.push(ContractOutputSelection::TransientStorageLayout);
        }
        selection
    }
}

impl ArtifactOutput for ConfigurableArtifacts {
    type Artifact = ConfigurableContractArtifact;
    type CompilerContract = Contract;

    /// Writes extra files for compiled artifact based on [Self::additional_files]
    fn handle_artifacts(
        &self,
        contracts: &crate::VersionedContracts<Contract>,
        artifacts: &crate::Artifacts<Self::Artifact>,
    ) -> Result<(), SolcError> {
        for (file, contracts) in contracts.as_ref().iter() {
            for (name, versioned_contracts) in contracts {
                for contract in versioned_contracts {
                    if let Some(artifact) = artifacts.find_artifact(file, name, &contract.version) {
                        let file = &artifact.file;
                        utils::create_parent_dir_all(file)?;
                        self.additional_files.write_extras(&contract.contract, file)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn contract_to_artifact(
        &self,
        _file: &Path,
        _name: &str,
        contract: Contract,
        source_file: Option<&SourceFile>,
    ) -> Self::Artifact {
        let mut artifact_userdoc = None;
        let mut artifact_devdoc = None;
        let mut artifact_raw_metadata = None;
        let mut artifact_metadata = None;
        let mut artifact_ir = None;
        let mut artifact_ir_optimized = None;
        let mut artifact_ir_optimized_ast = None;
        let mut artifact_ewasm = None;
        let mut artifact_bytecode = None;
        let mut artifact_deployed_bytecode = None;
        let mut artifact_gas_estimates = None;
        let mut artifact_function_debug_data = None;
        let mut artifact_method_identifiers = None;
        let mut artifact_assembly = None;
        let mut artifact_legacy_assembly = None;
        let mut artifact_storage_layout = None;
        let mut artifact_transient_storage_layout = None;
        let mut generated_sources = None;
        let mut opcodes = None;

        let Contract {
            abi,
            metadata,
            userdoc,
            devdoc,
            ir,
            storage_layout,
            transient_storage_layout,
            evm,
            ewasm,
            ir_optimized,
            ir_optimized_ast,
        } = contract;

        if self.additional_values.metadata {
            if let Some(LosslessMetadata { raw_metadata, metadata }) = metadata {
                artifact_raw_metadata = Some(raw_metadata);
                artifact_metadata = Some(metadata);
            }
        }
        if self.additional_values.userdoc {
            artifact_userdoc = Some(userdoc);
        }
        if self.additional_values.devdoc {
            artifact_devdoc = Some(devdoc);
        }
        if self.additional_values.ewasm {
            artifact_ewasm = ewasm;
        }
        if self.additional_values.ir {
            artifact_ir = ir;
        }
        if self.additional_values.ir_optimized {
            artifact_ir_optimized = ir_optimized;
        }
        if self.additional_values.ir_optimized_ast {
            artifact_ir_optimized_ast = ir_optimized_ast;
        }
        if self.additional_values.storage_layout {
            artifact_storage_layout = Some(storage_layout);
        }
        if self.additional_values.transient_storage_layout {
            artifact_transient_storage_layout = Some(transient_storage_layout);
        }

        if let Some(evm) = evm {
            let Evm {
                assembly,
                mut bytecode,
                deployed_bytecode,
                method_identifiers,
                gas_estimates,
                legacy_assembly,
            } = evm;

            if self.additional_values.function_debug_data {
                artifact_function_debug_data =
                    bytecode.as_mut().map(|code| std::mem::take(&mut code.function_debug_data));
            }
            if self.additional_values.generated_sources {
                generated_sources =
                    bytecode.as_mut().map(|code| std::mem::take(&mut code.generated_sources));
            }

            if self.additional_values.opcodes {
                opcodes = bytecode.as_mut().and_then(|code| code.opcodes.take())
            }

            artifact_bytecode = bytecode.map(Into::into);
            artifact_deployed_bytecode = deployed_bytecode.map(Into::into);
            artifact_method_identifiers = Some(method_identifiers);

            if self.additional_values.gas_estimates {
                artifact_gas_estimates = gas_estimates;
            }
            if self.additional_values.assembly {
                artifact_assembly = assembly;
            }

            if self.additional_values.legacy_assembly {
                artifact_legacy_assembly = legacy_assembly;
            }
        }

        ConfigurableContractArtifact {
            abi,
            bytecode: artifact_bytecode,
            deployed_bytecode: artifact_deployed_bytecode,
            assembly: artifact_assembly,
            legacy_assembly: artifact_legacy_assembly,
            opcodes,
            function_debug_data: artifact_function_debug_data,
            method_identifiers: artifact_method_identifiers,
            gas_estimates: artifact_gas_estimates,
            raw_metadata: artifact_raw_metadata,
            metadata: artifact_metadata,
            storage_layout: artifact_storage_layout,
            transient_storage_layout: artifact_transient_storage_layout,
            userdoc: artifact_userdoc,
            devdoc: artifact_devdoc,
            ir: artifact_ir,
            ir_optimized: artifact_ir_optimized,
            ir_optimized_ast: artifact_ir_optimized_ast,
            ewasm: artifact_ewasm,
            id: source_file.as_ref().map(|s| s.id),
            ast: source_file.and_then(|s| s.ast.clone()),
            generated_sources: generated_sources.unwrap_or_default(),
        }
    }

    fn standalone_source_file_to_artifact(
        &self,
        _path: &Path,
        file: &VersionedSourceFile,
    ) -> Option<Self::Artifact> {
        file.source_file.ast.clone().map(|ast| ConfigurableContractArtifact {
            abi: Some(JsonAbi::default()),
            id: Some(file.source_file.id),
            ast: Some(ast),
            bytecode: Some(CompactBytecode::empty()),
            deployed_bytecode: Some(CompactDeployedBytecode::empty()),
            ..Default::default()
        })
    }

    /// We want to enforce recompilation if artifact is missing data we need for writing extra
    /// files.
    fn is_dirty(&self, artifact_file: &ArtifactFile<Self::Artifact>) -> Result<bool, SolcError> {
        let artifact = &artifact_file.artifact;
        let ExtraOutputFiles {
            abi: _,
            metadata,
            ir,
            ir_optimized,
            ewasm,
            assembly,
            legacy_assembly,
            source_map,
            generated_sources,
            bytecode: _,
            deployed_bytecode: _,
            __non_exhaustive: _,
        } = self.additional_files;

        if metadata && artifact.metadata.is_none() {
            return Ok(true);
        }
        if ir && artifact.ir.is_none() {
            return Ok(true);
        }
        if ir_optimized && artifact.ir_optimized.is_none() {
            return Ok(true);
        }
        if ewasm && artifact.ewasm.is_none() {
            return Ok(true);
        }
        if assembly && artifact.assembly.is_none() {
            return Ok(true);
        }
        if assembly && artifact.assembly.is_none() {
            return Ok(true);
        }
        if legacy_assembly && artifact.legacy_assembly.is_none() {
            return Ok(true);
        }
        if source_map && artifact.get_source_map_str().is_none() {
            return Ok(true);
        }
        if generated_sources {
            // We can't check if generated sources are missing or just empty.
            return Ok(true);
        }
        Ok(false)
    }

    /// Writes extra files for cached artifacts based on [Self::additional_files].
    fn handle_cached_artifacts(
        &self,
        artifacts: &crate::Artifacts<Self::Artifact>,
    ) -> Result<(), SolcError> {
        for artifacts in artifacts.values() {
            for artifacts in artifacts.values() {
                for artifact_file in artifacts {
                    let file = &artifact_file.file;
                    let artifact = &artifact_file.artifact;
                    self.additional_files.process_abi(artifact.abi.as_ref(), file)?;
                    self.additional_files.process_assembly(artifact.assembly.as_deref(), file)?;
                    self.additional_files
                        .process_legacy_assembly(artifact.legacy_assembly.clone(), file)?;
                    self.additional_files
                        .process_bytecode(artifact.bytecode.as_ref().map(|b| &b.object), file)?;
                    self.additional_files.process_deployed_bytecode(
                        artifact
                            .deployed_bytecode
                            .as_ref()
                            .and_then(|d| d.bytecode.as_ref())
                            .map(|b| &b.object),
                        file,
                    )?;
                    self.additional_files
                        .process_generated_sources(Some(&artifact.generated_sources), file)?;
                    self.additional_files.process_ir(artifact.ir.as_deref(), file)?;
                    self.additional_files
                        .process_ir_optimized(artifact.ir_optimized.as_deref(), file)?;
                    self.additional_files.process_ewasm(artifact.ewasm.as_ref(), file)?;
                    self.additional_files.process_metadata(artifact.metadata.as_ref(), file)?;
                    self.additional_files
                        .process_source_map(artifact.get_source_map_str().as_deref(), file)?;
                }
            }
        }

        Ok(())
    }
}

/// Determines the additional values to include in the contract's artifact file
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtraOutputValues {
    pub ast: bool,
    pub userdoc: bool,
    pub devdoc: bool,
    pub method_identifiers: bool,
    pub storage_layout: bool,
    pub transient_storage_layout: bool,
    pub assembly: bool,
    pub legacy_assembly: bool,
    pub gas_estimates: bool,
    pub metadata: bool,
    pub ir: bool,
    pub ir_optimized: bool,
    pub ir_optimized_ast: bool,
    pub ewasm: bool,
    pub function_debug_data: bool,
    pub generated_sources: bool,
    pub source_map: bool,
    pub opcodes: bool,

    /// PRIVATE: This structure may grow, As such, constructing this structure should
    /// _always_ be done using a public constructor or update syntax:
    ///
    /// ```
    /// use foundry_compilers::ExtraOutputValues;
    ///
    /// let config = ExtraOutputValues { ir: true, ..Default::default() };
    /// ```
    #[doc(hidden)]
    pub __non_exhaustive: (),
}

impl ExtraOutputValues {
    /// Returns an instance where all values are set to `true`
    pub fn all() -> Self {
        Self {
            ast: true,
            userdoc: true,
            devdoc: true,
            method_identifiers: true,
            storage_layout: true,
            transient_storage_layout: true,
            assembly: true,
            legacy_assembly: true,
            gas_estimates: true,
            metadata: true,
            ir: true,
            ir_optimized: true,
            ir_optimized_ast: true,
            ewasm: true,
            function_debug_data: true,
            generated_sources: true,
            source_map: true,
            opcodes: true,
            __non_exhaustive: (),
        }
    }

    /// Sets the values based on a set of `ContractOutputSelection`
    pub fn from_output_selection(
        settings: impl IntoIterator<Item = ContractOutputSelection>,
    ) -> Self {
        let mut config = Self::default();
        for value in settings.into_iter() {
            match value {
                ContractOutputSelection::DevDoc => {
                    config.devdoc = true;
                }
                ContractOutputSelection::UserDoc => {
                    config.userdoc = true;
                }
                ContractOutputSelection::Metadata => {
                    config.metadata = true;
                }
                ContractOutputSelection::Ir => {
                    config.ir = true;
                }
                ContractOutputSelection::IrOptimized => {
                    config.ir_optimized = true;
                }
                ContractOutputSelection::StorageLayout => {
                    config.storage_layout = true;
                }
                ContractOutputSelection::Evm(evm) => match evm {
                    EvmOutputSelection::All => {
                        config.assembly = true;
                        config.legacy_assembly = true;
                        config.gas_estimates = true;
                        config.method_identifiers = true;
                        config.generated_sources = true;
                        config.source_map = true;
                        config.opcodes = true;
                    }
                    EvmOutputSelection::Assembly => {
                        config.assembly = true;
                    }
                    EvmOutputSelection::LegacyAssembly => {
                        config.legacy_assembly = true;
                    }
                    EvmOutputSelection::MethodIdentifiers => {
                        config.method_identifiers = true;
                    }
                    EvmOutputSelection::GasEstimates => {
                        config.gas_estimates = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::FunctionDebugData) => {
                        config.function_debug_data = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::Opcodes) => {
                        config.opcodes = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::GeneratedSources) => {
                        config.generated_sources = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::SourceMap) => {
                        config.source_map = true;
                    }
                    _ => {}
                },
                ContractOutputSelection::Ewasm(_) => {
                    config.ewasm = true;
                }
                ContractOutputSelection::IrOptimizedAst => {
                    config.ir_optimized_ast = true;
                }
                ContractOutputSelection::TransientStorageLayout => {
                    config.transient_storage_layout = true;
                }
                ContractOutputSelection::Abi => {}
            }
        }

        config
    }
}

/// Determines what to emit as an additional file
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtraOutputFiles {
    pub abi: bool,
    pub metadata: bool,
    pub ir: bool,
    pub ir_optimized: bool,
    pub ewasm: bool,
    pub assembly: bool,
    pub legacy_assembly: bool,
    pub source_map: bool,
    pub generated_sources: bool,
    pub bytecode: bool,
    pub deployed_bytecode: bool,

    /// PRIVATE: This structure may grow, As such, constructing this structure should
    /// _always_ be done using a public constructor or update syntax:
    ///
    /// ```
    /// use foundry_compilers::ExtraOutputFiles;
    ///
    /// let config = ExtraOutputFiles { metadata: true, ..Default::default() };
    /// ```
    #[doc(hidden)]
    pub __non_exhaustive: (),
}

impl ExtraOutputFiles {
    /// Returns an instance where all values are set to `true`
    pub fn all() -> Self {
        Self {
            abi: true,
            metadata: true,
            ir: true,
            ir_optimized: true,
            ewasm: true,
            assembly: true,
            legacy_assembly: true,
            source_map: true,
            generated_sources: true,
            bytecode: true,
            deployed_bytecode: true,
            __non_exhaustive: (),
        }
    }

    /// Sets the values based on a set of `ContractOutputSelection`
    pub fn from_output_selection(
        settings: impl IntoIterator<Item = ContractOutputSelection>,
    ) -> Self {
        let mut config = Self::default();
        for value in settings.into_iter() {
            match value {
                ContractOutputSelection::Abi => {
                    config.abi = true;
                }
                ContractOutputSelection::Metadata => {
                    config.metadata = true;
                }
                ContractOutputSelection::Ir => {
                    config.ir = true;
                }
                ContractOutputSelection::IrOptimized => {
                    config.ir_optimized = true;
                }
                ContractOutputSelection::Evm(evm) => match evm {
                    EvmOutputSelection::All => {
                        config.assembly = true;
                        config.legacy_assembly = true;
                        config.generated_sources = true;
                        config.source_map = true;
                        config.bytecode = true;
                        config.deployed_bytecode = true;
                    }
                    EvmOutputSelection::Assembly => {
                        config.assembly = true;
                    }
                    EvmOutputSelection::LegacyAssembly => {
                        config.legacy_assembly = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::GeneratedSources) => {
                        config.generated_sources = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::Object) => {
                        config.bytecode = true;
                    }
                    EvmOutputSelection::ByteCode(BytecodeOutputSelection::SourceMap) => {
                        config.source_map = true;
                    }
                    EvmOutputSelection::DeployedByteCode(DeployedBytecodeOutputSelection::All)
                    | EvmOutputSelection::DeployedByteCode(
                        DeployedBytecodeOutputSelection::Object,
                    ) => {
                        config.deployed_bytecode = true;
                    }
                    _ => {}
                },
                ContractOutputSelection::Ewasm(_) => {
                    config.ewasm = true;
                }
                _ => {}
            }
        }
        config
    }

    fn process_abi(&self, abi: Option<&JsonAbi>, file: &Path) -> Result<(), SolcError> {
        if self.abi {
            if let Some(abi) = abi {
                let file = file.with_extension("abi.json");
                fs::write(&file, serde_json::to_string_pretty(abi)?)
                    .map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_metadata(&self, metadata: Option<&Metadata>, file: &Path) -> Result<(), SolcError> {
        if self.metadata {
            if let Some(metadata) = metadata {
                let file = file.with_extension("metadata.json");
                fs::write(&file, serde_json::to_string_pretty(metadata)?)
                    .map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_ir(&self, ir: Option<&str>, file: &Path) -> Result<(), SolcError> {
        if self.ir {
            if let Some(ir) = ir {
                let file = file.with_extension("ir");
                fs::write(&file, ir).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_ir_optimized(
        &self,
        ir_optimized: Option<&str>,
        file: &Path,
    ) -> Result<(), SolcError> {
        if self.ir_optimized {
            if let Some(ir_optimized) = ir_optimized {
                let file = file.with_extension("iropt");
                fs::write(&file, ir_optimized).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_ewasm(&self, ewasm: Option<&Ewasm>, file: &Path) -> Result<(), SolcError> {
        if self.ewasm {
            if let Some(ewasm) = ewasm {
                let file = file.with_extension("ewasm");
                fs::write(&file, serde_json::to_vec_pretty(ewasm)?)
                    .map_err(|err| SolcError::io(err, file))?;
            }
        }
        Ok(())
    }

    fn process_assembly(&self, asm: Option<&str>, file: &Path) -> Result<(), SolcError> {
        if self.assembly {
            if let Some(asm) = asm {
                let file = file.with_extension("asm");
                fs::write(&file, asm).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_legacy_assembly(
        &self,
        asm: Option<serde_json::Value>,
        file: &Path,
    ) -> Result<(), SolcError> {
        if self.legacy_assembly {
            if let Some(legacy_asm) = asm {
                let file = file.with_extension("legacyAssembly.json");
                fs::write(&file, format!("{legacy_asm}")).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_generated_sources(
        &self,
        generated_sources: Option<&Vec<GeneratedSource>>,
        file: &Path,
    ) -> Result<(), SolcError> {
        if self.generated_sources {
            if let Some(generated_sources) = generated_sources {
                let file = file.with_extension("gensources");
                fs::write(&file, serde_json::to_vec_pretty(generated_sources)?)
                    .map_err(|err| SolcError::io(err, file))?;
            }
        }
        Ok(())
    }

    fn process_source_map(&self, source_map: Option<&str>, file: &Path) -> Result<(), SolcError> {
        if self.source_map {
            if let Some(source_map) = source_map {
                let file = file.with_extension("sourcemap");
                fs::write(&file, source_map).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_bytecode(
        &self,
        bytecode: Option<&BytecodeObject>,
        file: &Path,
    ) -> Result<(), SolcError> {
        if self.bytecode {
            if let Some(bytecode) = bytecode {
                let code = hex::encode(bytecode.as_ref());
                let file = file.with_extension("bin");
                fs::write(&file, code).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    fn process_deployed_bytecode(
        &self,
        deployed: Option<&BytecodeObject>,
        file: &Path,
    ) -> Result<(), SolcError> {
        if self.deployed_bytecode {
            if let Some(deployed) = deployed {
                let code = hex::encode(deployed.as_ref());
                let file = file.with_extension("deployed-bin");
                fs::write(&file, code).map_err(|err| SolcError::io(err, file))?
            }
        }
        Ok(())
    }

    /// Write the set values as separate files
    pub fn write_extras(&self, contract: &Contract, file: &Path) -> Result<(), SolcError> {
        self.process_abi(contract.abi.as_ref(), file)?;
        self.process_metadata(contract.metadata.as_ref().map(|m| &m.metadata), file)?;
        self.process_ir(contract.ir.as_deref(), file)?;
        self.process_ir_optimized(contract.ir_optimized.as_deref(), file)?;
        self.process_ewasm(contract.ewasm.as_ref(), file)?;

        let evm = contract.evm.as_ref();
        self.process_assembly(evm.and_then(|evm| evm.assembly.as_deref()), file)?;
        self.process_legacy_assembly(evm.and_then(|evm| evm.legacy_assembly.clone()), file)?;

        let bytecode = evm.and_then(|evm| evm.bytecode.as_ref());
        self.process_generated_sources(bytecode.map(|b| &b.generated_sources), file)?;

        let deployed_bytecode = evm.and_then(|evm| evm.deployed_bytecode.as_ref());
        self.process_source_map(bytecode.and_then(|b| b.source_map.as_deref()), file)?;
        self.process_bytecode(bytecode.map(|b| &b.object), file)?;
        self.process_deployed_bytecode(
            deployed_bytecode.and_then(|d| d.bytecode.as_ref()).map(|b| &b.object),
            file,
        )?;

        Ok(())
    }
}
