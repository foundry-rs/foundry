use foundry_common::{TestFilter, FunctionFilter, TestFunctionExt};
use alloy_json_abi::{Function, JsonAbi as Abi};


#[derive(Default)]
pub struct ArtifactFilter {

}

impl ArtifactFilter {


    pub fn get_artifact_functions<A: FunctionFilter>(
        self,
        filter: &A,
        
    ) -> impl Iterator<Item = &String> {
        
    }
}