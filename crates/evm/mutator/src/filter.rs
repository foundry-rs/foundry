use foundry_common::FunctionFilter;
use alloy_json_abi::Function;


#[derive(Default)]
pub struct ArtifactFilter {

}

impl ArtifactFilter {

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