use foundry_compilers_artifacts::Contract;

use crate::artifacts::vyper::{VyperCompilationError, VyperOutput};

impl From<VyperOutput> for super::CompilerOutput<VyperCompilationError, Contract> {
    fn from(output: VyperOutput) -> Self {
        Self {
            errors: output.errors,
            contracts: output
                .contracts
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().map(|(k, v)| (k, v.into())).collect()))
                .collect(),
            sources: output.sources.into_iter().map(|(k, v)| (k, v.into())).collect(),
            metadata: Default::default(),
        }
    }
}
