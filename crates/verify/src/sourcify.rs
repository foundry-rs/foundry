use crate::{
    provider::{
        ExternalVerificationContext, VerificationContext, VerificationProvider,
        VerificationProviderType,
    },
    utils::ensure_solc_build_metadata,
    verify::{ContractLanguage, VerifyArgs, VerifyCheckArgs},
};
use alloy_primitives::Address;
use async_trait::async_trait;
use eyre::{Context, Result, eyre};
use foundry_common::retry::RetryError;
use futures::{FutureExt, StreamExt};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::{future::Future as StdFuture, pin::Pin, time::Duration};
use url::Url;

pub static SOURCIFY_URL: &str = "https://sourcify.dev/server/";

/// The type that can verify a contract on `sourcify`
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SourcifyVerificationProvider;

#[async_trait]
impl VerificationProvider for SourcifyVerificationProvider {
    fn provider_type(&self) -> VerificationProviderType {
        VerificationProviderType::Sourcify
    }

    async fn preflight_verify_check(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<()> {
        let _ = self.prepare_verify_request(&args, &context).await?;
        Ok(())
    }

    async fn submit(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<Option<VerifyCheckArgs>> {
        let body = self.prepare_verify_request(&args, &context).await?;
        self.submit_verify_request(args, body, &context.target_name).await
    }

    fn submit_external(
        &mut self,
        args: VerifyArgs,
        context: ExternalVerificationContext,
    ) -> Pin<Box<dyn StdFuture<Output = Result<Option<VerifyCheckArgs>>> + '_>> {
        Box::pin(async move {
            let target = context.target.clone();
            let body = Self::prepare_external_verify_request(&args, context);
            self.submit_verify_request(args, body, &target).await
        })
    }

    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let url = Self::get_job_status_url(args.verifier.verifier_url.as_deref(), args.id.clone());

        args.retry
            .into_retry()
            .run_async_until_break(|| async {
                let response = verification_client()
                    .map_err(RetryError::Break)?
                    .get(&url)
                    .send()
                    .await
                    .wrap_err("Failed to request verification status")
                    .map_err(RetryError::Retry)?;

                if response.status() == StatusCode::NOT_FOUND {
                    return Err(RetryError::Break(eyre!(
                        "No verification job found for ID {}",
                        sanitize_remote_message(&args.id)
                    )));
                }

                if !response.status().is_success() {
                    return Err(RetryError::Retry(eyre!(
                        "Failed to request verification status with status code {}",
                        response.status()
                    )));
                }

                let job_response: SourcifyJobResponse = serde_json::from_slice(
                    &read_capped_body(response).await.map_err(RetryError::Retry)?,
                )
                .wrap_err("Failed to parse job response")
                .map_err(RetryError::Retry)?;

                if !job_response.is_job_completed {
                    return Err(RetryError::Retry(eyre!("Verification is still pending...")));
                }

                if let Some(error) = job_response.error {
                    if error.custom_code == "already_verified" {
                        let _ = sh_status!("Contract source code already verified");
                        return Ok(());
                    }

                    return Err(RetryError::Break(eyre!(
                        "Verification job failed:\nError Code: `{}`\nMessage: `{}`",
                        sanitize_remote_message(&error.custom_code),
                        sanitize_remote_message(&error.message)
                    )));
                }

                if let Some(contract_status) = job_response.contract.match_status {
                    let _ = sh_status!(
                        "Contract successfully verified:\nStatus: `{}`",
                        sanitize_remote_message(&contract_status),
                    );
                }
                Ok(())
            })
            .await
            .wrap_err("Checking verification result failed")
    }
}

fn sanitize_remote_message(message: &str) -> String {
    message.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).take(512).collect()
}

const MAX_RESPONSE_BODY: usize = 1024 * 1024;

fn verification_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .wrap_err("Failed to create Sourcify HTTP client")
}

async fn read_capped_body(response: reqwest::Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        eyre::ensure!(
            body.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BODY,
            "Sourcify response body exceeds 1 MiB"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl SourcifyVerificationProvider {
    fn prepare_external_verify_request(
        args: &VerifyArgs,
        context: ExternalVerificationContext,
    ) -> SourcifyVerifyRequest {
        SourcifyVerifyRequest {
            std_json_input: (*context.standard_json_input).clone(),
            compiler_version: context.compiler_version.to_string(),
            contract_identifier: context.target,
            creation_transaction_hash: args.creation_transaction_hash.map(|hash| hash.to_string()),
        }
    }

    async fn submit_verify_request(
        &self,
        args: VerifyArgs,
        body: SourcifyVerifyRequest,
        target: &str,
    ) -> Result<Option<VerifyCheckArgs>> {
        if !args.skip_is_verified_check && self.is_contract_verified(&args).await? {
            sh_status!(
                "Contract [{}] {:?} is already verified. Skipping verification.",
                target,
                args.address.to_string()
            )?;

            return Ok(None);
        }

        trace!(provider = "Sourcify", address = %args.address, target, "submitting verification request");

        let chain_id = args.etherscan.chain.unwrap_or_default().id();
        let url =
            Self::get_verify_url(args.verifier.verifier_url.as_deref(), chain_id, args.address);
        let client = verification_client()?;

        let resp = args
            .retry
            .into_retry()
            .run_async(|| {
                async {
                    sh_status!("Submitting verification for [{}] {}.", target, args.address)?;
                    let response = client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .body(serde_json::to_string(&body)?)
                        .send()
                        .await?;

                    let status = response.status();
                    match status {
                        StatusCode::CONFLICT => {
                            sh_status!("Contract source code already fully verified")?;
                            Ok(None)
                        }
                        StatusCode::ACCEPTED => {
                            let text = read_capped_body(response).await?;
                            let verify_response: SourcifyVerificationResponse =
                                serde_json::from_slice(&text)
                                    .wrap_err("Failed to parse Sourcify verification response")?;
                            Ok(Some(verify_response))
                        }
                        _ => {
                            eyre::bail!(
                                "Sourcify verification request for address ({}) failed with status code {status}",
                                args.address,
                            );
                        }
                    }
                }
                .boxed()
            })
            .await?;

        if let Some(resp) = resp {
            eyre::ensure!(
                !resp.verification_id.is_empty()
                    && resp.verification_id.len() <= 512
                    && !matches!(resp.verification_id.as_str(), "." | ".."),
                "Sourcify returned an invalid verification job ID"
            );
            let display_id = sanitize_remote_message(&resp.verification_id);
            let job_url = Self::get_job_ui_url(
                args.verifier.verifier_url.as_deref(),
                resp.verification_id.clone(),
            );
            sh_status!(
                "Submitted contract for verification:\n\tVerification Job ID: `{}`\n\tURL: {}",
                display_id,
                job_url
            )?;
            if args.print_submission_result_to_stdout {
                sh_println!("{}\t{}", display_id, job_url)?;
            }
            Ok(Some(VerifyCheckArgs {
                id: resp.verification_id,
                etherscan: args.etherscan,
                retry: args.retry,
                verifier: args.verifier,
            }))
        } else {
            Ok(None)
        }
    }

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
        Self::get_job_url(verifier_url, &["v2", "verify"], &job_id)
    }

    fn get_job_ui_url(verifier_url: Option<&str>, job_id: String) -> String {
        Self::get_job_url(verifier_url, &["verify-ui", "jobs"], &job_id)
    }

    fn get_job_url(verifier_url: Option<&str>, path: &[&str], job_id: &str) -> String {
        let base_url = Self::get_base_url(verifier_url);
        let job_id = encode_path_segment(job_id);
        format!("{base_url}{}/{job_id}", path.join("/"))
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
                let input = context.get_solc_standard_json_input()?;

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
                let input = context.get_vyper_standard_json_input()?;
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

        match verification_client()?.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let contract_response: SourcifyContractResponse =
                        serde_json::from_slice(&read_capped_body(response).await?)
                            .wrap_err("Failed to parse contract response")?;

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

fn encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
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
    use foundry_config::Config;
    use foundry_test_utils::forgetest_async;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn external_request_preserves_raw_input_and_target() {
        let args =
            VerifyArgs::parse_from(["foundry-cli", "0xd8509bee9c9bf012282ad33aba0d87241baf5064"]);
        let input = json!({
            "language": "Solidity",
            "settings": { "unknownSetting": { "futureField": true } },
            "unknownTopLevel": [1, 2, 3]
        });
        let context = ExternalVerificationContext {
            config: Config::default(),
            compiler_version: "0.8.30+commit.73712a01".parse().unwrap(),
            standard_json_input: std::sync::Arc::new(input.clone()),
            target: "contracts/Unknown.sol:ExactTarget".to_string(),
        };

        let request = SourcifyVerificationProvider::prepare_external_verify_request(&args, context);
        assert_eq!(request.std_json_input, input);
        assert_eq!(request.contract_identifier, "contracts/Unknown.sol:ExactTarget");
        assert_eq!(request.compiler_version, "0.8.30+commit.73712a01");
    }

    #[test]
    fn job_urls_encode_opaque_ids_as_one_path_segment() {
        let id = "job a+b/part?query#fragment\n".to_string();
        let status = SourcifyVerificationProvider::get_job_status_url(None, id.clone());
        let ui = SourcifyVerificationProvider::get_job_ui_url(None, id);
        let encoded = "job%20a%2Bb%2Fpart%3Fquery%23fragment%0A";
        assert!(status.ends_with(&format!("v2/verify/{encoded}")), "{status}");
        assert!(ui.ends_with(&format!("verify-ui/jobs/{encoded}")), "{ui}");
        assert_eq!(encode_path_segment("."), "%2E");
        assert_eq!(encode_path_segment(".."), "%2E%2E");
    }

    #[tokio::test]
    async fn verification_client_follows_redirects() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_url = format!("http://{}", target.local_addr().unwrap());
        let target_thread = thread::spawn(move || {
            let (mut socket, _) = target.accept().unwrap();
            let mut request = [0; 4096];
            let bytes_read = socket.read(&mut request).unwrap();
            assert!(std::str::from_utf8(&request[..bytes_read]).unwrap().starts_with("GET "));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nredirected",
                )
                .unwrap();
        });

        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let source_url = format!("http://{}", source.local_addr().unwrap());
        let source_thread = thread::spawn(move || {
            let (mut socket, _) = source.accept().unwrap();
            let mut request = [0; 4096];
            let bytes_read = socket.read(&mut request).unwrap();
            assert!(bytes_read > 0);
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
        });

        let response = verification_client()
            .unwrap()
            .get(source_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        source_thread.join().unwrap();
        target_thread.join().unwrap();
        assert_eq!(response, "redirected");
    }

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
