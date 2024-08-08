use crate::{
    provider::{VerificationContext, VerificationProvider},
    verify::{VerifyArgs, VerifyCheckArgs},
};
use async_trait::async_trait;
use eyre::Result;
use foundry_common::{fs, retry::Retry};
use futures::FutureExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};

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
        let _ = self.prepare_request(&args, &context)?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs, context: VerificationContext) -> Result<()> {
        let body = self.prepare_request(&args, &context)?;

        trace!("submitting verification request {:?}", body);

        let client = reqwest::Client::new();

        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| {
                async {
                    println!(
                        "\nSubmitting verification for [{}] {:?}.",
                        context.target_name,
                        args.address.to_string()
                    );
                    let response = client
                        .post(args.verifier.verifier_url.as_deref().unwrap_or(SOURCIFY_URL))
                        .header("Content-Type", "application/json")
                        .body(serde_json::to_string(&body)?)
                        .send()
                        .await?;

                    let status = response.status();
                    if !status.is_success() {
                        let error: serde_json::Value = response.json().await?;
                        eyre::bail!(
                            "Sourcify verification request for address ({}) failed with status code {status}\nDetails: {error:#}",
                            args.address,
                        );
                    }

                    let text = response.text().await?;
                    Ok(Some(serde_json::from_str::<SourcifyVerificationResponse>(&text)?))
                }
                .boxed()
            })
            .await?;

        self.process_sourcify_response(resp.map(|r| r.result))
    }

    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| {
                async {
                    let url = Url::from_str(
                        args.verifier.verifier_url.as_deref().unwrap_or(SOURCIFY_URL),
                    )?;
                    let query = format!(
                        "check-by-addresses?addresses={}&chainIds={}",
                        args.id,
                        args.etherscan.chain.unwrap_or_default().id(),
                    );
                    let url = url.join(&query)?;
                    let response = reqwest::get(url).await?;
                    if !response.status().is_success() {
                        eyre::bail!(
                            "Failed to request verification status with status code {}",
                            response.status()
                        );
                    };

                    Ok(Some(response.json::<Vec<SourcifyResponseElement>>().await?))
                }
                .boxed()
            })
            .await?;

        self.process_sourcify_response(resp)
    }
}

impl SourcifyVerificationProvider {
    /// Configures the API request to the sourcify API using the given [`VerifyArgs`].
    fn prepare_request(
        &self,
        args: &VerifyArgs,
        context: &VerificationContext,
    ) -> Result<SourcifyVerifyRequest> {
        let metadata = context.get_target_metadata()?;
        let imports = context.get_target_imports()?;

        let mut files = HashMap::with_capacity(2 + imports.len());

        let metadata = serde_json::to_string_pretty(&metadata)?;
        files.insert("metadata.json".to_string(), metadata);

        let contract_path = context.target_path.clone();
        let filename = contract_path.file_name().unwrap().to_string_lossy().to_string();
        files.insert(filename, fs::read_to_string(&contract_path)?);

        for import in imports {
            let import_entry = format!("{}", import.display());
            files.insert(import_entry, fs::read_to_string(&import)?);
        }

        let req = SourcifyVerifyRequest {
            address: args.address.to_string(),
            chain: args.etherscan.chain.unwrap_or_default().id().to_string(),
            files,
            chosen_contract: None,
        };

        Ok(req)
    }

    fn process_sourcify_response(
        &self,
        response: Option<Vec<SourcifyResponseElement>>,
    ) -> Result<()> {
        let Some([response, ..]) = response.as_deref() else { return Ok(()) };
        match response.status.as_str() {
            "perfect" => {
                if let Some(ts) = &response.storage_timestamp {
                    println!("Contract source code already verified. Storage Timestamp: {ts}");
                } else {
                    println!("Contract successfully verified");
                }
            }
            "partial" => {
                println!("The recompiled contract partially matches the deployed version");
            }
            "false" => println!("Contract source code is not verified"),
            s => eyre::bail!("Unknown status from sourcify. Status: {s:?}"),
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct SourcifyVerifyRequest {
    address: String,
    chain: String,
    files: HashMap<String, String>,
    #[serde(rename = "chosenContract", skip_serializing_if = "Option::is_none")]
    chosen_contract: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SourcifyVerificationResponse {
    result: Vec<SourcifyResponseElement>,
}

#[derive(Debug, Deserialize)]
pub struct SourcifyResponseElement {
    status: String,
    #[serde(rename = "storageTimestamp")]
    storage_timestamp: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_addresses_url() {
        let url = Url::from_str("https://server-verify.hashscan.io").unwrap();
        let url = url.join("check-by-addresses?addresses=0x1234&chainIds=1").unwrap();
        assert_eq!(
            url.as_str(),
            "https://server-verify.hashscan.io/check-by-addresses?addresses=0x1234&chainIds=1"
        );
    }
}
