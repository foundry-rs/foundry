use foundry_compilers_artifacts::{CompilerOutput, SolcInput};
use foundry_compilers_core::error::Result;

use crate::compilers::solc::Solc;

/// The result of a `solc` process bundled with its `Solc` and `CompilerInput`
type CompileElement = (Result<CompilerOutput>, Solc, SolcInput);

/// The bundled output of multiple `solc` processes.
#[derive(Debug)]
pub struct CompiledMany {
    outputs: Vec<CompileElement>,
}

impl CompiledMany {
    pub fn new(outputs: Vec<CompileElement>) -> Self {
        Self { outputs }
    }

    /// Returns an iterator over all output elements
    pub fn outputs(&self) -> impl Iterator<Item = &CompileElement> {
        self.outputs.iter()
    }

    /// Returns an iterator over all output elements
    pub fn into_outputs(self) -> impl Iterator<Item = CompileElement> {
        self.outputs.into_iter()
    }

    /// Returns all `CompilerOutput` or the first error that occurred
    pub fn flattened(self) -> Result<Vec<CompilerOutput>> {
        self.into_iter().collect()
    }
}

impl IntoIterator for CompiledMany {
    type Item = Result<CompilerOutput>;
    type IntoIter = std::vec::IntoIter<Result<CompilerOutput>>;

    fn into_iter(self) -> Self::IntoIter {
        self.outputs.into_iter().map(|(res, _, _)| res).collect::<Vec<_>>().into_iter()
    }
}
