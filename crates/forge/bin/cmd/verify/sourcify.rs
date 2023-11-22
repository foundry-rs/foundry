use super::{provider::VerificationProvider, VerifyArgs, VerifyCheckArgs};
use async_trait::async_trait;
use eyre::Result;
use foundry_cli::utils::{get_cached_entry_by_name, LoadConfig};
use foundry_common::{fs, Retry};
use foundry_compilers::ConfigurableContractArtifact;
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

pub static SOURCIFY_URL: &str = "https://sourcify.dev/server/";

/// The type that can verify a contract on `sourcify`
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct SourcifyVerificationProvider;

#[async_trait]
impl VerificationProvider for SourcifyVerificationProvider {
    async fn preflight_check(&mut self, args: VerifyArgs) -> Result<()> {
        let _ = self.prepare_request(&args)?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs) -> Result<()> {
        let body = self.prepare_request(&args)?;

        trace!("submitting verification request {:?}", body);

        let client = reqwest::Client::new();

        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| {
                async {
                    println!(
                        "\nSubmitting verification for [{}] {:?}.",
                        args.contract.name,
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
                        eprintln!(
                            "Sourcify verification request for address ({}) failed with status code {}\nDetails: {:#}",
                            format_args!("{:?}", args.address),
                            status,
                            error
                        );
                        warn!("Failed verify submission: {:?}", error);
                        std::process::exit(1);
                    }

                    let text = response.text().await?;
                    Ok(Some(serde_json::from_str::<SourcifyVerificationResponse>(&text)?))
                }
                .boxed()
            })
            .await?;

        self.process_sourcify_response(resp.map(|r| r.result));
        Ok(())
    }

    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| {
                async {
                    let url = format!(
                        "{}check-by-addresses?addresses={}&chainIds={}",
                        args.verifier.verifier_url.as_deref().unwrap_or(SOURCIFY_URL),
                        args.id,
                        args.etherscan.chain.unwrap_or_default().id(),
                    );

                    let response = reqwest::get(url).await?;
                    if !response.status().is_success() {
                        eprintln!(
                            "Failed to request verification status with status code {}",
                            response.status()
                        );
                        std::process::exit(1);
                    };

                    Ok(Some(response.json::<Vec<SourcifyResponseElement>>().await?))
                }
                .boxed()
            })
            .await?;

        self.process_sourcify_response(resp);
        Ok(())
    }
}

impl SourcifyVerificationProvider {
    /// Configures the API request to the sourcify API using the given [`VerifyArgs`].
    fn prepare_request(&self, args: &VerifyArgs) -> Result<SourcifyVerifyRequest> {
        let mut config = args.try_load_config_emit_warnings()?;
        config.libraries.extend(args.libraries.clone());

        let project = config.project()?;

        if !config.cache {
            eyre::bail!("Cache is required for sourcify verification.")
        }

        let cache = project.read_cache_file()?;
        let (path, entry) = get_cached_entry_by_name(&cache, &args.contract.name)?;

        if entry.solc_config.settings.metadata.is_none() {
            eyre::bail!(
                r#"Contract {} was compiled without the solc `metadata` setting.
Sourcify requires contract metadata for verification.
metadata output can be enabled via `extra_output = ["metadata"]` in `foundry.toml`"#,
                args.contract.name
            )
        }

        let mut files = HashMap::with_capacity(2 + entry.imports.len());

        // the metadata is included in the contract's artifact file
        let artifact_path = entry
            .find_artifact_path(&args.contract.name)
            .ok_or_else(|| eyre::eyre!("No artifact found for contract {}", args.contract.name))?;

        let artifact: ConfigurableContractArtifact = fs::read_json_file(artifact_path)?;
        if let Some(metadata) = artifact.metadata {
            let metadata = serde_json::to_string_pretty(&metadata)?;
            files.insert("metadata.json".to_string(), metadata);
        } else {
            eyre::bail!(
                r#"No metadata found in artifact `{}` for contract {}.
Sourcify requires contract metadata for verification.
metadata output can be enabled via `extra_output = ["metadata"]` in `foundry.toml`"#,
                artifact_path.display(),
                args.contract.name
            )
        }

        let contract_path = args.contract.path.clone().map_or(path, PathBuf::from);
        let filename = contract_path.file_name().unwrap().to_string_lossy().to_string();
        files.insert(filename, fs::read_to_string(&contract_path)?);

        for import in entry.imports {
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

    fn process_sourcify_response(&self, response: Option<Vec<SourcifyResponseElement>>) {
        let response = response.unwrap().remove(0);
        if response.status == "perfect" {
            if let Some(ts) = response.storage_timestamp {
                println!("Contract source code already verified. Storage Timestamp: {ts}");
            } else {
                println!("Contract successfully verified")
            }
        } else if response.status == "partial" {
            println!("The recompiled contract partially matches the deployed version")
        } else if response.status == "false" {
            println!("Contract source code is not verified")
        } else {
            eprintln!("Unknown status from sourcify. Status: {}", response.status);
            std::process::exit(1);
        }
    }
}

#[derive(Serialize, Debug)]
pub struct SourcifyVerifyRequest {
    address: String,
    chain: String,
    files: HashMap<String, String>,
    #[serde(rename = "chosenContract", skip_serializing_if = "Option::is_none")]
    chosen_contract: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct SourcifyVerificationResponse {
    result: Vec<SourcifyResponseElement>,
}

#[derive(Deserialize, Debug)]
pub struct SourcifyResponseElement {
    status: String,
    #[serde(rename = "storageTimestamp")]
    storage_timestamp: Option<String>,
}
