use super::{ScriptArgs, ScriptConfig};
use alloy_primitives::{Address, Bytes};
use eyre::{Context, ContextCompat, Result};
use forge::link::{LinkOutput, Linker};
use foundry_cli::utils::get_cached_entry_by_name;
use foundry_common::compile::{self, ContractSources, ProjectCompiler};
use foundry_compilers::{
    artifacts::{ContractBytecode, ContractBytecodeSome, Libraries},
    cache::SolFilesCache,
    contracts::ArtifactContracts,
    info::ContractInfo,
    ArtifactId, Project, ProjectCompileOutput,
};
use std::str::FromStr;

impl ScriptArgs {
    /// Compiles the file or project and the verify metadata.
    pub fn compile(&mut self, script_config: &mut ScriptConfig) -> Result<BuildOutput> {
        trace!(target: "script", "compiling script");

        self.build(script_config)
    }

    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&mut self, script_config: &mut ScriptConfig) -> Result<BuildOutput> {
        let (project, output) = self.get_project_and_output(script_config)?;
        let root = project.root();
        let output = output.with_stripped_file_prefixes(root);
        let sources = ContractSources::from_project_output(&output, root)?;
        let contracts = output.into_artifacts().collect();

        let target = self.find_target(&project, &contracts)?.clone();
        script_config.target_contract = Some(target.clone());

        let libraries = script_config.config.libraries_with_remappings()?;
        let linker = Linker::new(project.root(), contracts);

        let (highlevel_known_contracts, libraries, predeploy_libraries) = self.link_script_target(
            &linker,
            libraries,
            script_config.evm_opts.sender,
            script_config.sender_nonce,
            target.clone(),
        )?;

        let contract = highlevel_known_contracts.get(&target).unwrap();

        Ok(BuildOutput {
            project,
            linker,
            contract: contract.clone(),
            highlevel_known_contracts,
            libraries,
            predeploy_libraries,
            sources,
        })
    }

    /// Tries to find artifact for the target script contract.
    pub fn find_target<'a>(
        &self,
        project: &Project,
        contracts: &'a ArtifactContracts,
    ) -> Result<&'a ArtifactId> {
        let mut target_fname = dunce::canonicalize(&self.path)
            .wrap_err("Couldn't convert contract path to absolute path.")?
            .strip_prefix(project.root())
            .wrap_err("Couldn't strip project root from contract path.")?
            .to_str()
            .wrap_err("Bad path to string.")?
            .to_string();

        let no_target_name = if let Some(target_name) = &self.target_contract {
            target_fname = target_fname + ":" + target_name;
            false
        } else {
            true
        };

        let mut target: Option<&ArtifactId> = None;

        for (id, contract) in contracts.iter() {
            if no_target_name {
                // Match artifact source, and ignore interfaces
                if id.source == std::path::Path::new(&target_fname) &&
                    contract.bytecode.as_ref().map_or(false, |b| b.object.bytes_len() > 0)
                {
                    if let Some(target) = target {
                        // We might have multiple artifacts for the same contract but with different
                        // solc versions. Their names will have form of {name}.0.X.Y, so we are
                        // stripping versions off before comparing them.
                        let target_name = target.name.split('.').next().unwrap();
                        let id_name = id.name.split('.').next().unwrap();
                        if target_name != id_name {
                            eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`")
                        }
                    }
                    target = Some(id);
                }
            } else {
                let (path, name) =
                    target_fname.rsplit_once(':').expect("The target specifier is malformed.");
                let path = std::path::Path::new(path);
                if path == id.source && name == id.name {
                    target = Some(id);
                }
            }
        }

        target.ok_or_else(|| eyre::eyre!("Could not find target contract: {}", target_fname))
    }

    /// Links script artifact with given libraries or library addresses computed from script sender
    /// and nonce.
    ///
    /// Populates [BuildOutput] with linked target contract, libraries, bytes of libs that need to
    /// be predeployed and `highlevel_known_contracts` - set of known fully linked contracts
    pub fn link_script_target(
        &self,
        linker: &Linker,
        libraries: Libraries,
        sender: Address,
        nonce: u64,
        target: ArtifactId,
    ) -> Result<(ArtifactContracts<ContractBytecodeSome>, Libraries, Vec<Bytes>)> {
        let LinkOutput { libs_to_deploy, libraries } =
            linker.link_with_nonce_or_address(libraries, sender, nonce, &target)?;

        // Collect all linked contracts with non-empty bytecode
        let highlevel_known_contracts = linker
            .get_linked_artifacts(&libraries)?
            .iter()
            .filter_map(|(id, contract)| {
                ContractBytecodeSome::try_from(ContractBytecode::from(contract.clone()))
                    .ok()
                    .map(|tc| (id.clone(), tc))
            })
            .filter(|(_, tc)| tc.bytecode.object.is_non_empty_bytecode())
            .collect();

        Ok((highlevel_known_contracts, libraries, libs_to_deploy))
    }

    pub fn get_project_and_output(
        &mut self,
        script_config: &ScriptConfig,
    ) -> Result<(Project, ProjectCompileOutput)> {
        let project = script_config.config.project()?;

        let filters = self.opts.skip.clone().unwrap_or_default();
        // We received a valid file path.
        // If this file does not exist, `dunce::canonicalize` will
        // result in an error and it will be handled below.
        if let Ok(target_contract) = dunce::canonicalize(&self.path) {
            let output = compile::compile_target_with_filter(
                &target_contract,
                &project,
                self.opts.args.silent,
                self.verify,
                filters,
            )?;
            return Ok((project, output))
        }

        if !project.paths.has_input_files() {
            eyre::bail!("The project doesn't have any input files. Make sure the `script` directory is configured properly in foundry.toml. Otherwise, provide the path to the file.")
        }

        let contract = ContractInfo::from_str(&self.path)?;
        self.target_contract = Some(contract.name.clone());

        // We received `contract_path:contract_name`
        if let Some(path) = contract.path {
            let path =
                dunce::canonicalize(path).wrap_err("Could not canonicalize the target path")?;
            let output = compile::compile_target_with_filter(
                &path,
                &project,
                self.opts.args.silent,
                self.verify,
                filters,
            )?;
            self.path = path.to_string_lossy().to_string();
            return Ok((project, output))
        }

        // We received `contract_name`, and need to find its file path.
        let output = ProjectCompiler::new().compile(&project)?;
        let cache =
            SolFilesCache::read_joined(&project.paths).wrap_err("Could not open compiler cache")?;

        let (path, _) = get_cached_entry_by_name(&cache, &contract.name)
            .wrap_err("Could not find target contract in cache")?;
        self.path = path.to_string_lossy().to_string();

        Ok((project, output))
    }
}

pub struct BuildOutput {
    pub project: Project,
    pub contract: ContractBytecodeSome,
    pub linker: Linker,
    pub highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    pub libraries: Libraries,
    pub predeploy_libraries: Vec<Bytes>,
    pub sources: ContractSources,
}
