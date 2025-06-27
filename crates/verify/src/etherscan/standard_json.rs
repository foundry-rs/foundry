use super::{EtherscanSourceProvider, VerifyArgs};
use crate::provider::VerificationContext;
use eyre::{Context, Result};
use foundry_block_explorers::verify::CodeFormat;
use foundry_compilers::{artifacts::StandardJsonCompilerInput, solc::SolcLanguage};
use crate::verify::{detect_language, ContractLanguage};

#[derive(Debug)]
pub struct EtherscanStandardJsonSource;
impl EtherscanSourceProvider for EtherscanStandardJsonSource {
    fn source(
        &self,
        _args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<(String, String, CodeFormat)> {
        let mut input: StandardJsonCompilerInput = context
            .project
            .standard_json_input(&context.target_path)
            .wrap_err("Failed to get standard json input")?
            .normalize_evm_version(&context.compiler_version);

        let lang = detect_language(_args, context);

        let code_format = match lang {
            ContractLanguage::Solidity => CodeFormat::StandardJsonInput,
            ContractLanguage::Vyper => CodeFormat::VyperJson,
        };

        let mut settings = context.compiler_settings.solc.settings.clone();
        settings.libraries.libs = input
            .settings
            .libraries
            .libs
            .into_iter()
            .map(|(f, libs)| {
                (f.strip_prefix(context.project.root()).unwrap_or(&f).to_path_buf(), libs)
            })
            .collect();

        settings.remappings = input.settings.remappings;

        // remove all incompatible settings
        settings.sanitize(&context.compiler_version, SolcLanguage::Solidity);

        input.settings = settings;

        let source =
            serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;

        trace!(target: "forge::verify", standard_json=source, "determined standard json input");

        let name = format!(
            "{}:{}",
            context
                .target_path
                .strip_prefix(context.project.root())
                .unwrap_or(context.target_path.as_path())
                .display(),
            context.target_name.clone()
        );
        Ok((source, name, code_format))
    }
}
