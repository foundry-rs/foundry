use std::{collections::{HashMap, BTreeMap}, sync::{mpsc::Sender, Arc}};
use foundry_cli::utils::FoundryPathExt;
use eyre::{eyre, ErrReport, Result};
use foundry_compilers::{remappings::RelativeRemapping, FileFilter, Artifact, ArtifactOutput, ProjectCompileOutput, ConfigurableArtifacts, ConfigurableContractArtifact, ArtifactId};
pub use gambit::Mutant;
use gambit::{run_mutate, MutateParams, Mutator as GambitMutator};
use itertools::Itertools;
use std::path::{Path, PathBuf};
use foundry_common::{TestFilter, FunctionFilter, TestFunctionExt};
use alloy_json_abi::{Function, JsonAbi as Abi};


const DEFAULT_GAMBIT_DIR_OUT: &'static str = "gambit_out";

pub type GambitArtifacts = Vec<(ArtifactId, Abi)>;

#[derive(Debug, Clone)]
pub struct MutatorConfigBuilder {
    solc: PathBuf,
    solc_allow_paths: Vec<PathBuf>,
    solc_include_paths: Vec<PathBuf>,
    solc_remappings: Vec<RelativeRemapping>,
    solc_optimize: bool,
}

impl MutatorConfigBuilder {
    pub fn new(
        solc: PathBuf,
        solc_optimize: bool,
        solc_allow_paths: Vec<PathBuf>,
        solc_include_paths: Vec<PathBuf>,
        solc_remappings: Vec<RelativeRemapping>,
    ) -> Self {
        Self {solc, solc_allow_paths, solc_include_paths, solc_remappings, solc_optimize }
    }

    pub fn build<A: ArtifactOutput>(
        self,
        root: impl AsRef<Path>,
        src_folder_root: PathBuf,
        output: ProjectCompileOutput<A>,
    ) -> Result<Mutator> {
        // Converts the compiled output into artifactId and abi
        // It does not include files with .t.sol extension
        let artifacts: Vec<(ArtifactId, Abi)> = output
            // .with_stripped_file_prefixes(&root)
            .into_artifacts()
            .filter_map(|(id, c)| match (id.source.as_path().is_sol_test(), c.into_abi()) {
                (false, Some(b)) => Some((id, b)),
                _ => None
            })
            .collect::<Vec<(ArtifactId, Abi)>>();

        let solc = self.solc.to_str().ok_or(eyre!("failed to decode solc root"))?;
        let solc_allow_paths: Vec<String> = self
            .solc_allow_paths
            .into_iter()
            .filter_map(|x| x.to_str().map(|x| x.to_string()))
            .collect();
        let solc_include_paths: String = self
            .solc_include_paths
            .into_iter()
            .filter_map(|x| x.to_str().map(|x| x.to_string()))
            .join(",");
        let solc_remappings: Vec<String> =
            self.solc_remappings.into_iter().map(|x| x.to_string()).collect();
        let source_root = src_folder_root.to_str().ok_or(eyre!("failed to decode source root"))?;

        Ok(Mutator::new(
            artifacts,
            source_root.to_owned(),
            solc.to_owned(),
            solc_allow_paths,
            solc_include_paths,
            solc_remappings,
            self.solc_optimize,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct Mutator {
    artifacts: GambitArtifacts,
    default_mutate_params: MutateParams,
}

impl Mutator {
    pub fn new(
        artifacts: GambitArtifacts,
        source_root: String,
        solc: String,
        solc_allow_paths: Vec<String>,
        solc_include_paths: String,
        solc_remappings: Vec<String>,
        solc_optimize: bool,
    ) -> Self {
        // create mutate params here
        let default_mutate_params = MutateParams {
            json: None,
            filename: None,
            num_mutants: None,
            random_seed: false,
            seed: 0,
            outdir: Some(DEFAULT_GAMBIT_DIR_OUT.into()),
            sourceroot: Some(source_root.into()),
            mutations: None,
            no_export: false,
            no_overwrite: false,
            solc: solc.into(),
            solc_optimize,
            functions: None,
            contract: None,
            solc_base_path: None,
            solc_allow_paths: Some(solc_allow_paths.into()),
            solc_include_path: Some(solc_include_paths.into()),
            solc_remappings: Some(solc_remappings.into()),
            skip_validate: false,
        };

        Self { artifacts, default_mutate_params }
    }

    /// Returns the number of matching functions
    pub fn matching_function_count<A : TestFilter + FunctionFilter>(&self, filter: &A) -> usize {
        self.matching_functions(filter).count()
    }

    /// Returns all functions matching the filter
    pub fn matching_functions<'a, A>(&'a self, filter: &'a A) -> impl Iterator<Item = &Function> 
        where A: TestFilter + FunctionFilter 
    {
        self.artifacts.iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy())
                &&
                filter.matches_contract(&id.name)
            })
            .flat_map(|(_, abi)| abi.functions())
    }

    /// Returns the name of the functions to generate Mutants
    pub fn get_artifact_functions<'a, A>(&'a self, artifact_id: &'a ArtifactId, filter: &'a A) -> impl Iterator<Item = &String> 
        where A: TestFilter + FunctionFilter 
    {
        self.artifacts.iter()
            .filter_map(|(id, abi)| match id.clone() == artifact_id.clone() {
                true => Some(abi.functions().collect::<Vec<_>>()),
                false => None
            })
            .flatten()
            .filter_map(|func: &Function| match filter.matches_function(&func.name) {
                true => Some(&func.name),
                false => None
            })

    }


    pub fn filtered_functions<'a, A>(&'a self, filter: &'a A) -> impl Iterator<Item = &Function> 
        where A: TestFilter + FunctionFilter 
    {
        self.artifacts
            .iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy()) &&
                    filter.matches_contract(&id.name)
            })
            .flat_map(|(_, abi)| abi.functions())
    }

    pub fn get_functions<'a, A>(&'a self, filter: &'a A) -> Vec<String> 
        where A: TestFilter + FunctionFilter 
    {
        self.filtered_functions(filter)
            .map(|func| func.name.clone())
            .filter(|name| !name.is_test())
            .collect()
    }

    /// Returns all matching functions grouped by contract 
    /// grouped by file (file -> contract -> functions)
    pub fn list<A : TestFilter + FunctionFilter>(
        &self,
        filter: &A
    ) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        self.artifacts
            .iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy()) &&
                    filter.matches_contract(&id.name)
                && !id.source.as_path().is_sol_test()
            })
            .map(|(id, abi)| {
                let source = id.source.as_path().display().to_string();
                let name = id.name.clone();
                let functions = abi.functions()
                    .filter(|func| !func.name.is_test())
                    .filter(|func| filter.matches_function(func.name.clone()))
                    .map(|func| func.name.clone())
                    .collect::<Vec<_>>();
                println!("source {:?}", source);
                (source, name , functions)
            })
            .fold( BTreeMap::new(), | mut acc, (source, name, functions) | {
                acc.entry(source).or_default().insert(name, functions);
                acc
            })
    }
    
    pub fn run_mutate<A>(
        self,
        root: impl AsRef<Path>,
        filter: A
    ) -> Result<HashMap<String, Vec<Mutant>>>
        where A : TestFilter + FunctionFilter
    {
        let mutant_params = self.artifacts
            .iter()
            .filter(|(id, abi)| {
                id.source.starts_with(&root)
                &&
                filter.matches_path(id.source.to_string_lossy()) 
                &&
                filter.matches_contract(&id.name)
                &&
                abi.functions().any(|func| filter.matches_function(&func.name))
            })
            .map(|(id, abi)| {
                let mut current_mutate_params = self.default_mutate_params.clone();
                current_mutate_params.outdir = Some(id.name.clone());
                current_mutate_params.functions = Some(self.get_artifact_functions(id, &filter).map(|x| x.clone()).collect());
                current_mutate_params.filename = Some(String::from(id.source.to_str().expect("failed run mutate filename")));
                current_mutate_params.contract = Some(String::from(id.name.clone()));
                current_mutate_params
            })
            .collect_vec();

        run_mutate(mutant_params).map_err(|err| eyre!("{:?}", err))

    }
}
