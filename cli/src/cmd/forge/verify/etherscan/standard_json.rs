use super::{EtherscanSourceProvider, VerifyArgs};
use ethers::{
    etherscan::verify::CodeFormat,
    prelude::artifacts::StandardJsonCompilerInput,
    solc::{CompilerInput, Project},
};
use eyre::Context;
use semver::Version;
use std::path::Path;
use tracing::trace;

#[derive(Debug)]
pub struct EtherscanStandardJsonSource;
impl EtherscanSourceProvider for EtherscanStandardJsonSource {
    fn source(
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> eyre::Result<(String, String, CodeFormat)> {
        let mut input: StandardJsonCompilerInput = project
            .standard_json_input(target)
            .wrap_err("Failed to get standard json input")?
            .normalize_evm_version(version);

        input.settings.libraries.libs = input
            .settings
            .libraries
            .libs
            .into_iter()
            .map(|(f, libs)| (f.strip_prefix(project.root()).unwrap_or(&f).to_path_buf(), libs))
            .collect();

        // TODO: make sanitization logic shared between types in ethers
        let input: StandardJsonCompilerInput = CompilerInput::from(input).sanitized(version).into();

        let source =
            serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;

        trace!(target : "forge::verify",  standard_json = source, "determined standard json input");

        let name = format!(
            "{}:{}",
            target.strip_prefix(project.root()).unwrap_or(target).display(),
            args.contract.name.clone()
        );
        Ok((source, name, CodeFormat::StandardJsonInput))
    }
}
