mod runner;
pub use runner::{ContractRunner, TestKind, TestKindGas, TestResult};

mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

#[cfg(test)]
pub mod test_helpers {
    use ethers::{
        prelude::Lazy,
        solc::{CompilerOutput, Project, ProjectPathsConfig},
    };

    pub static COMPILED: Lazy<CompilerOutput> = Lazy::new(|| {
        // NB: should we add a test-helper function that makes creating these
        // ephemeral projects easier?
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        project.compile().unwrap().output()
    });
}
