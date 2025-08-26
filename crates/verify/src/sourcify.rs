use crate::{
    provider::{VerificationContext, VerificationProvider},
    retry::RETRY_CHECK_ON_VERIFY,
    utils::ensure_solc_build_metadata,
    verify::{ContractLanguage, VerifyArgs, VerifyCheckArgs},
};
use async_trait::async_trait;
use eyre::{Context, Result, eyre};
use foundry_common::retry::RetryError;
use foundry_compilers::{
    artifacts::{Source, StandardJsonCompilerInput, vyper::VyperInput},
    solc::SolcLanguage,
};
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub static SOURCIFY_URL: &str = "https://sourcify.dev/server/";

/// The type that can verify a contract on `sourcify`
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SourcifyVerificationProvider;

#[async_trait]
impl VerificationProvider for SourcifyVerificationProvider {
    async fn preflight_verify_check(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<()> {
        let _ = self.prepare_verify_request(&args, &context).await?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs, context: VerificationContext) -> Result<()> {
        let body = self.prepare_verify_request(&args, &context).await?;
        let chain_id = args.etherscan.chain.unwrap_or_default().id();

        if !args.skip_is_verified_check && self.is_contract_verified(&args).await? {
            sh_println!(
                "\nContract [{}] {:?} is already verified. Skipping verification.",
                context.target_name,
                args.address.to_string()
            )?;

            return Ok(());
        }

        trace!("submitting verification request {:?}", body);

        let client = reqwest::Client::new();
        let base_url = args.verifier.verifier_url.as_deref().unwrap_or(SOURCIFY_URL);
        let url = format!("{}v2/verify/{}/{}", base_url, chain_id, args.address);

        let resp = args
            .retry
            .into_retry()
            .run_async(|| {
                async {
                    sh_println!(
                        "\nSubmitting verification for [{}] {:?}.",
                        context.target_name,
                        args.address.to_string()
                    )?;
                    let response = client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .body(serde_json::to_string(&body)?)
                        .send()
                        .await?;

                    let status = response.status();

                    if status == 409 {
                        sh_println!("Contract source code already fully verified")?;
                        return Ok(None);
                    }

                    if status == 202 {
                        let text = response.text().await?;
                        let verify_response: SourcifyVerificationResponse =
                            serde_json::from_str(&text)
                                .wrap_err("Failed to parse Sourcify verification response")?;
                        return Ok(Some(verify_response));
                    }

                    let error: serde_json::Value = response.json().await?;
                    eyre::bail!(
                        "Sourcify verification request for address ({}) \
                            failed with status code {status}\n\
                            Details: {error:#}",
                        args.address,
                    );
                }
                .boxed()
            })
            .await?;

        if let Some(resp) = resp {
            let job_url = format!("{}v2/verify/{}", base_url, resp.verification_id);
            sh_println!(
                "Submitted contract for verification:\n\tVerification Job ID: `{}`\n\tURL: {}",
                resp.verification_id,
                job_url
            )?;

            if args.watch {
                let check_args = VerifyCheckArgs {
                    id: resp.verification_id,
                    etherscan: args.etherscan,
                    retry: RETRY_CHECK_ON_VERIFY,
                    verifier: args.verifier,
                };
                return self.check(check_args).await;
            }
        }

        Ok(())
    }

    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let base_url = args.verifier.verifier_url.as_deref().unwrap_or(SOURCIFY_URL);
        let url = format!("{}v2/verify/{}", base_url, args.id);

        args.retry
            .into_retry()
            .run_async_until_break(|| async {
                let response = reqwest::get(&url)
                    .await
                    .wrap_err("Failed to request verification status")
                    .map_err(RetryError::Retry)?;

                if response.status() == 404 {
                    return Err(RetryError::Break(eyre!(
                        "No verification job found for ID {}",
                        args.id
                    )));
                }

                if !response.status().is_success() {
                    return Err(RetryError::Retry(eyre!(
                        "Failed to request verification status with status code {}",
                        response.status()
                    )));
                }

                let job_response: SourcifyJobResponse = response
                    .json()
                    .await
                    .wrap_err("Failed to parse job response")
                    .map_err(RetryError::Retry)?;

                if !job_response.is_job_completed {
                    return Err(RetryError::Retry(eyre!("Verification is still pending...")));
                }

                if let Some(error) = job_response.error {
                    if error.custom_code == "already_verified" {
                        let _ = sh_println!("Contract source code already verified");
                        return Ok(());
                    }

                    return Err(RetryError::Break(eyre!(
                        "Verification job failed:\nError Code: `{}`\nMessage: `{}`",
                        error.custom_code,
                        error.message
                    )));
                }

                if let Some(contract_status) = job_response.contract.match_status {
                    let _ = sh_println!(
                        "Contract successfully verified:\nStatus: `{}`",
                        contract_status,
                    );
                }
                Ok(())
            })
            .await
            .wrap_err("Checking verification result failed")
    }
}

impl SourcifyVerificationProvider {
    /// Configures the API request to the sourcify API using the given [`VerifyArgs`].
    async fn prepare_verify_request(
        &self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<SourcifyVerifyRequest> {
        let lang = args.detect_language(context);

        let std_json_input = match lang {
            ContractLanguage::Solidity => {
                let mut input: StandardJsonCompilerInput = context
                    .project
                    .standard_json_input(&context.target_path)
                    .wrap_err("Failed to get standard json input")?
                    .normalize_evm_version(&context.compiler_version);

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

                serde_json::to_value(&input).wrap_err("Failed to serialize standard json input")?
            }
            ContractLanguage::Vyper => {
                let path = Path::new(&context.target_path);
                let sources = Source::read_all_from(path, &["vy", "vyi"])?;
                let input = VyperInput::new(
                    sources,
                    context.clone().compiler_settings.vyper,
                    &context.compiler_version,
                );

                serde_json::to_value(&input).wrap_err("Failed to serialize vyper json input")?
            }
        };

        let contract_identifier = format!(
            "{}:{}",
            context
                .target_path
                .strip_prefix(context.project.root())
                .unwrap_or(context.target_path.as_path())
                .display(),
            context.target_name
        );

        let compiler_version = if matches!(lang, ContractLanguage::Vyper) {
            context
                .compiler_version
                .clone()
                .to_string()
                .split('+')
                .next()
                .unwrap_or("0.0.0")
                .to_string()
        } else {
            ensure_solc_build_metadata(context.compiler_version.clone()).await?.to_string()
        };

        let req = SourcifyVerifyRequest {
            std_json_input,
            compiler_version,
            contract_identifier,
            creation_transaction_hash: args.creation_transaction_hash.map(|h| h.to_string()),
        };

        Ok(req)
    }

    async fn is_contract_verified(&self, args: &VerifyArgs) -> Result<bool> {
        let chain_id = args.etherscan.chain.unwrap_or_default().id();
        let base_url = args.verifier.verifier_url.as_deref().unwrap_or(SOURCIFY_URL);
        let url = format!("{}v2/contract/{}/{}", base_url, chain_id, args.address);

        match reqwest::get(&url).await {
            Ok(response) => {
                if response.status() == 200 {
                    let contract_response: SourcifyContractResponse =
                        response.json().await.wrap_err("Failed to parse contract response")?;

                    let creation_exact = contract_response
                        .creation_match
                        .as_ref()
                        .map(|s| s == "exact_match")
                        .unwrap_or(false);

                    let runtime_exact = contract_response
                        .runtime_match
                        .as_ref()
                        .map(|s| s == "exact_match")
                        .unwrap_or(false);

                    Ok(creation_exact && runtime_exact)
                } else {
                    Ok(false)
                }
            }
            Err(error) => Err(error).wrap_err_with(|| {
                format!("Failed to query verification status for {}", args.address)
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SourcifyVerifyRequest {
    #[serde(rename = "stdJsonInput")]
    std_json_input: serde_json::Value,
    #[serde(rename = "compilerVersion")]
    compiler_version: String,
    #[serde(rename = "contractIdentifier")]
    contract_identifier: String,
    #[serde(rename = "creationTransactionHash", skip_serializing_if = "Option::is_none")]
    creation_transaction_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SourcifyVerificationResponse {
    #[serde(rename = "verificationId")]
    verification_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SourcifyJobResponse {
    #[serde(rename = "isJobCompleted")]
    is_job_completed: bool,
    contract: SourcifyContractResponse,
    error: Option<SourcifyErrorResponse>,
}

#[derive(Debug, Deserialize)]
pub struct SourcifyContractResponse {
    #[serde(rename = "match")]
    match_status: Option<String>,
    #[serde(rename = "creationMatch")]
    creation_match: Option<String>,
    #[serde(rename = "runtimeMatch")]
    runtime_match: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SourcifyErrorResponse {
    #[serde(rename = "customCode")]
    custom_code: String,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use foundry_test_utils::forgetest_async;

    forgetest_async!(creates_correct_verify_request_body, |prj, _cmd| {
        prj.add_source("Counter", "contract Counter {}").unwrap();

        let args = VerifyArgs::parse_from([
            "foundry-cli",
            "0xd8509bee9c9bf012282ad33aba0d87241baf5064",
            "src/Counter.sol:Counter",
            "--compiler-version",
            "0.8.19",
            "--root",
            &prj.root().to_string_lossy(),
        ]);

        let context = args.resolve_context().await.unwrap();
        let provider = SourcifyVerificationProvider::default();
        let request = provider.prepare_verify_request(&args, &context).await.unwrap();

        assert_eq!(request.compiler_version, "0.8.19+commit.7dd6d404");
        assert_eq!(request.contract_identifier, "src/Counter.sol:Counter");
        assert!(request.creation_transaction_hash.is_none());

        assert!(request.std_json_input.is_object());
        let json_obj = request.std_json_input.as_object().unwrap();
        assert!(json_obj.contains_key("sources"));
        assert!(json_obj.contains_key("settings"));

        let sources = json_obj.get("sources").unwrap().as_object().unwrap();
        assert!(sources.contains_key("src/Counter.sol"));
        let counter_source = sources.get("src/Counter.sol").unwrap().as_object().unwrap();
        let content = counter_source.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("contract Counter {}"));
    });
}
