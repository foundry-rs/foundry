use crate::{
    provider::{VerificationContext, VerificationProvider},
    retry::RETRY_CHECK_ON_VERIFY,
    utils::ensure_solc_build_metadata,
    verify::{ContractLanguage, VerifyArgs, VerifyCheckArgs},
};
use alloy_primitives::Address;
use async_trait::async_trait;
use eyre::{Context, Result, eyre};
use foundry_common::retry::RetryError;
use foundry_compilers::{
    artifacts::{Source, StandardJsonCompilerInput, vyper::VyperInput},
    solc::SolcLanguage,
};
use futures::FutureExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::path::Path;
use url::Url;

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
        let url =
            Self::get_verify_url(args.verifier.verifier_url.as_deref(), chain_id, args.address);

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
                    match status {
                        StatusCode::CONFLICT => {
                            sh_println!("Contract source code already fully verified")?;
                            Ok(None)
                        }
                        StatusCode::ACCEPTED => {
                            let text = response.text().await?;
                            let verify_response: SourcifyVerificationResponse =
                                serde_json::from_str(&text)
                                    .wrap_err("Failed to parse Sourcify verification response")?;
                            Ok(Some(verify_response))
                        }
                        _ => {
                            let error: serde_json::Value = response.json().await?;
                            eyre::bail!(
                                "Sourcify verification request for address ({}) \
                            failed with status code {status}\n\
                            Details: {error:#}",
                                args.address,
                            );
                        }
                    }
                }
                .boxed()
            })
            .await?;

        if let Some(resp) = resp {
            let job_url = Self::get_job_status_url(
                args.verifier.verifier_url.as_deref(),
                resp.verification_id.clone(),
            );
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
        let url = Self::get_job_status_url(args.verifier.verifier_url.as_deref(), args.id.clone());

        args.retry
            .into_retry()
            .run_async_until_break(|| async {
                let response = reqwest::get(&url)
                    .await
                    .wrap_err("Failed to request verification status")
                    .map_err(RetryError::Retry)?;

                if response.status() == StatusCode::NOT_FOUND {
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
    fn get_base_url(verifier_url: Option<&str>) -> Url {
        // note(onbjerg): a little ugly but makes this infallible as we guarantee `SOURCIFY_URL` to
        // be well formatted
        Url::parse(verifier_url.unwrap_or(SOURCIFY_URL))
            .unwrap_or_else(|_| Url::parse(SOURCIFY_URL).unwrap())
    }

    fn get_verify_url(
        verifier_url: Option<&str>,
        chain_id: u64,
        contract_address: Address,
    ) -> String {
        let base_url = Self::get_base_url(verifier_url);
        format!("{base_url}v2/verify/{chain_id}/{contract_address}")
    }

    fn get_job_status_url(verifier_url: Option<&str>, job_id: String) -> String {
        let base_url = Self::get_base_url(verifier_url);
        format!("{base_url}v2/verify/{job_id}")
    }

    fn get_lookup_url(
        verifier_url: Option<&str>,
        chain_id: u64,
        contract_address: Address,
    ) -> String {
        let base_url = Self::get_base_url(verifier_url);
        format!("{base_url}v2/contract/{chain_id}/{contract_address}")
    }

    /// Configures the API request to the sourcify API using the given [`VerifyArgs`].
    async fn prepare_verify_request(
        &self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<SourcifyVerifyRequest> {
        let lang = args.detect_language(context);
        let contract_identifier = format!(
            "{}:{}",
            context
                .target_path
                .strip_prefix(context.project.root())
                .unwrap_or(context.target_path.as_path())
                .display(),
            context.target_name
        );
        let creation_transaction_hash = args.creation_transaction_hash.map(|h| h.to_string());

        match lang {
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

                let std_json_input = serde_json::to_value(&input)
                    .wrap_err("Failed to serialize standard json input")?;
                let compiler_version =
                    ensure_solc_build_metadata(context.compiler_version.clone()).await?.to_string();

                Ok(SourcifyVerifyRequest {
                    std_json_input,
                    compiler_version,
                    contract_identifier,
                    creation_transaction_hash,
                })
            }
            ContractLanguage::Vyper => {
                let path = Path::new(&context.target_path);
                let sources = Source::read_all_from(path, &["vy", "vyi"])?;
                let input = VyperInput::new(
                    sources,
                    context.clone().compiler_settings.vyper,
                    &context.compiler_version,
                );
                let std_json_input = serde_json::to_value(&input)
                    .wrap_err("Failed to serialize vyper json input")?;

                let compiler_version = context.compiler_version.to_string();

                Ok(SourcifyVerifyRequest {
                    std_json_input,
                    compiler_version,
                    contract_identifier,
                    creation_transaction_hash,
                })
            }
        }
    }

    async fn is_contract_verified(&self, args: &VerifyArgs) -> Result<bool> {
        let chain_id = args.etherscan.chain.unwrap_or_default().id();
        let url =
            Self::get_lookup_url(args.verifier.verifier_url.as_deref(), chain_id, args.address);

        match reqwest::get(&url).await {
            Ok(response) => {
                if response.status().is_success() {
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
#[serde(rename_all = "camelCase")]
pub struct SourcifyVerifyRequest {
    std_json_input: serde_json::Value,
    compiler_version: String,
    contract_identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    creation_transaction_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcifyVerificationResponse {
    verification_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcifyJobResponse {
    is_job_completed: bool,
    contract: SourcifyContractResponse,
    error: Option<SourcifyErrorResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcifyContractResponse {
    #[serde(rename = "match")]
    match_status: Option<String>,
    creation_match: Option<String>,
    runtime_match: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcifyErrorResponse {
    custom_code: String,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use foundry_test_utils::forgetest_async;

    forgetest_async!(creates_correct_verify_request_body, |prj, _cmd| {
        prj.add_source("Counter", "contract Counter {}");

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
