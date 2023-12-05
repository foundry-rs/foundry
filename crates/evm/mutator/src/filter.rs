use std::path::PathBuf;
use foundry_cli::utils::FoundryPathExt;
use foundry_common::{FunctionFilter, TestFilter};
use foundry_compilers::ArtifactId;
use alloy_json_abi::Function;


#[derive(Default)]
pub struct ArtifactFilter {
    // filter: MutatorFilterArgs
}

impl ArtifactFilter {
    /// Returns if an artifact is a valid mutator source file
    /// Return true for files in the config `src` directory
    /// It is false if the file extension is .t.sol
    pub fn valid_mutator_source_file<'a>(
        self,
        root: &'a PathBuf,
        artifact_id: &'a ArtifactId
    ) -> bool {
        // @TODO discuss if this is required or necessary
        artifact_id.source.starts_with(&root)
        &&
        !artifact_id.source.as_path().is_sol_test()
    }

    /// Returns if a artifact matches the required filter
    pub fn matching_artifact<'a, A: TestFilter + FunctionFilter>(
        self,
        root: &'a PathBuf,
        id: &'a ArtifactId,
        filter: &'a A
    ) -> bool {
        self.valid_mutator_source_file(root, id)
        &&
        filter.matches_path(id.source.to_string_lossy()) 
        &&
        filter.matches_contract(&id.name)
    }

    /// Returns the name of the functions to generate Mutants
    pub fn get_artifact_functions<'a, A: FunctionFilter>(
        self,
        filter: &'a A,
        functions: &'a[Function]
    ) -> impl Iterator<Item = &'a String> {
        functions.iter().filter_map(
            |func| filter.matches_function(&func.name).then_some(&func.name)
        )
    }

}

