use super::*;
use alloy_primitives::{Address, Bytes};
use eyre::{Context, ContextCompat, Result};
use foundry_cli::utils::get_cached_entry_by_name;
use foundry_common::{
    compact_to_contract,
    compile::{self, ContractSources},
    fs,
};
use foundry_compilers::{
    artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome, Libraries},
    cache::SolFilesCache,
    contracts::ArtifactContracts,
    info::ContractInfo,
    ArtifactId, Project, ProjectCompileOutput,
};
use foundry_utils::{PostLinkInput, ResolvedDependency};
use std::{collections::BTreeMap, str::FromStr};
use tracing::{trace, warn};

impl ScriptArgs {
    /// Compiles the file or project and the verify metadata.
    pub fn compile(&mut self, script_config: &mut ScriptConfig) -> Result<BuildOutput> {
        trace!(target: "script", "compiling script");

        self.build(script_config)
    }

    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&mut self, script_config: &mut ScriptConfig) -> Result<BuildOutput> {
        let (project, output) = self.get_project_and_output(script_config)?;
        let output = output.with_stripped_file_prefixes(project.root());

        let mut sources: ContractSources = Default::default();

        let contracts = output
            .into_artifacts()
            .map(|(id, artifact)| -> Result<_> {
                // Sources are only required for the debugger, but it *might* mean that there's
                // something wrong with the build and/or artifacts.
                if let Some(source) = artifact.source_file() {
                    let abs_path = source
                        .ast
                        .ok_or(eyre::eyre!("Source from artifact has no AST."))?
                        .absolute_path;
                    let source_code = fs::read_to_string(abs_path).wrap_err_with(|| {
                        format!("Failed to read artifact source file for `{}`", id.identifier())
                    })?;
                    let contract = artifact.clone().into_contract_bytecode();
                    let source_contract = compact_to_contract(contract)?;
                    sources
                        .0
                        .entry(id.clone().name)
                        .or_default()
                        .insert(source.id, (source_code, source_contract));
                } else {
                    warn!("source not found for artifact={:?}", id);
                }
                Ok((id, artifact))
            })
            .collect::<Result<ArtifactContracts>>()?;

        let mut output = self.link(
            project,
            contracts,
            script_config.config.parsed_libraries()?,
            script_config.evm_opts.sender,
            script_config.sender_nonce,
        )?;

        output.sources = sources;
        script_config.target_contract = Some(output.target.clone());

        Ok(output)
    }

    pub fn link(
        &self,
        project: Project,
        contracts: ArtifactContracts,
        libraries_addresses: Libraries,
        sender: Address,
        nonce: u64,
    ) -> Result<BuildOutput> {
        let mut run_dependencies = vec![];
        let mut contract = CompactContractBytecode::default();
        let mut highlevel_known_contracts = BTreeMap::new();

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

        let mut extra_info = ExtraLinkingInfo {
            no_target_name,
            target_fname: target_fname.clone(),
            contract: &mut contract,
            dependencies: &mut run_dependencies,
            matched: false,
            target_id: None,
        };

        // link_with_nonce_or_address expects absolute paths
        let mut libs = libraries_addresses.clone();
        for (file, libraries) in libraries_addresses.libs.iter() {
            if file.is_relative() {
                let mut absolute_path = project.root().clone();
                absolute_path.push(file);
                libs.libs.insert(absolute_path, libraries.clone());
            }
        }

        foundry_utils::link_with_nonce_or_address(
            contracts.clone(),
            &mut highlevel_known_contracts,
            libs,
            sender,
            nonce,
            &mut extra_info,
            |post_link_input| {
                let PostLinkInput {
                    contract,
                    known_contracts: highlevel_known_contracts,
                    id,
                    extra,
                    dependencies,
                } = post_link_input;

                fn unique_deps(deps: Vec<ResolvedDependency>) -> Vec<(String, Bytes)> {
                    let mut filtered = Vec::new();
                    let mut seen = HashSet::new();
                    for dep in deps {
                        if !seen.insert(dep.id.clone()) {
                            continue
                        }
                        filtered.push((dep.id, dep.bytecode));
                    }

                    filtered
                }

                // if it's the target contract, grab the info
                if extra.no_target_name {
                    // Match artifact source, and ignore interfaces
                    if id.source == std::path::Path::new(&extra.target_fname) &&
                        contract.bytecode.as_ref().map_or(false, |b| b.object.bytes_len() > 0)
                    {
                        if extra.matched {
                            eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`")
                        }
                        *extra.dependencies = unique_deps(dependencies);
                        *extra.contract = contract.clone();
                        extra.matched = true;
                        extra.target_id = Some(id.clone());
                    }
                } else {
                    let (path, name) = extra
                        .target_fname
                        .rsplit_once(':')
                        .expect("The target specifier is malformed.");
                    let path = std::path::Path::new(path);
                    if path == id.source && name == id.name {
                        *extra.dependencies = unique_deps(dependencies);
                        *extra.contract = contract.clone();
                        extra.matched = true;
                        extra.target_id = Some(id.clone());
                    }
                }

                if let Ok(tc) = ContractBytecode::from(contract).try_into() {
                    highlevel_known_contracts.insert(id, tc);
                }

                Ok(())
            },
            project.root(),
        )?;

        let target = extra_info
            .target_id
            .ok_or_else(|| eyre::eyre!("Could not find target contract: {}", target_fname))?;

        let (new_libraries, predeploy_libraries): (Vec<_>, Vec<_>) =
            run_dependencies.into_iter().unzip();

        // Merge with user provided libraries
        let mut new_libraries = Libraries::parse(&new_libraries)?;
        for (file, libraries) in libraries_addresses.libs.into_iter() {
            new_libraries.libs.entry(file).or_default().extend(libraries)
        }

        Ok(BuildOutput {
            target,
            contract,
            known_contracts: contracts,
            highlevel_known_contracts: ArtifactContracts(highlevel_known_contracts),
            predeploy_libraries,
            sources: Default::default(),
            project,
            libraries: new_libraries,
        })
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
        let output = if self.opts.args.silent {
            compile::suppress_compile(&project)
        } else {
            compile::compile(&project, false, false)
        }?;
        let cache =
            SolFilesCache::read_joined(&project.paths).wrap_err("Could not open compiler cache")?;

        let (path, _) = get_cached_entry_by_name(&cache, &contract.name)
            .wrap_err("Could not find target contract in cache")?;
        self.path = path.to_string_lossy().to_string();

        Ok((project, output))
    }
}

struct ExtraLinkingInfo<'a> {
    no_target_name: bool,
    target_fname: String,
    contract: &'a mut CompactContractBytecode,
    dependencies: &'a mut Vec<(String, Bytes)>,
    matched: bool,
    target_id: Option<ArtifactId>,
}

pub struct BuildOutput {
    pub project: Project,
    pub target: ArtifactId,
    pub contract: CompactContractBytecode,
    pub known_contracts: ArtifactContracts,
    pub highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    pub libraries: Libraries,
    pub predeploy_libraries: Vec<Bytes>,
    pub sources: ContractSources,
}
