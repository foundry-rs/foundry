use super::{EtherscanSourceProvider, VerifyArgs};
use crate::{provider::VerificationContext, verify::ContractLanguage};
use eyre::{Context, Result};
use foundry_block_explorers::verify::CodeFormat;

#[derive(Debug)]
pub struct EtherscanStandardJsonSource;
impl EtherscanSourceProvider for EtherscanStandardJsonSource {
    fn source(
        &self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<(String, String, CodeFormat)> {
        let lang = args.detect_language(context);

        let code_format = match lang {
            ContractLanguage::Solidity => CodeFormat::StandardJsonInput,
            ContractLanguage::Vyper => CodeFormat::VyperJson,
        };

        let source = match lang {
            ContractLanguage::Solidity => {
                let input = context.get_solc_standard_json_input()?;
                serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?
            }
            ContractLanguage::Vyper => {
                let input = context.get_vyper_standard_json_input()?;
                serde_json::to_string(&input).wrap_err("Failed to parse vyper json input")?
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
