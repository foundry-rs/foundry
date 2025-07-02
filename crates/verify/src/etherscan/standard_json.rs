use super::{EtherscanSourceProvider, VerifyArgs};
use crate::provider::VerificationContext;
use crate::verify::ContractLanguage;
use eyre::{Context, Result};
use foundry_block_explorers::verify::CodeFormat;
use foundry_compilers::{artifacts::StandardJsonCompilerInput, solc::SolcLanguage};
use std::process::Command;

#[derive(Debug)]
pub struct EtherscanStandardJsonSource;
impl EtherscanSourceProvider for EtherscanStandardJsonSource {
    fn source(
        &self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<(String, String, CodeFormat)> {
        let mut input: StandardJsonCompilerInput = context
            .project
            .standard_json_input(&context.target_path)
            .wrap_err("Failed to get standard json input")?
            .normalize_evm_version(&context.compiler_version);

        let lang = args.detect_language(context);

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

        let source = match lang {
            ContractLanguage::Solidity => {
                serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?
            }
            ContractLanguage::Vyper => {
                // Execute vyper -f solc_json {target_path}
                let output = Command::new("vyper")
                    .arg("-f")
                    .arg("solc_json")
                    .arg(&context.target_path)
                    .output()
                    .wrap_err("Failed to execute vyper command")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(eyre::eyre!("Vyper command failed: {}", stderr));
                }

                String::from_utf8(output.stdout)
                    .wrap_err("Failed to parse vyper output as UTF-8")?
            }
        };

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
