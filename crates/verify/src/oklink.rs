use super::{provider::VerificationProvider, VerifyArgs, VerifyCheckArgs};
use async_trait::async_trait;
use eyre::Result;
use foundry_cli::utils::{get_cached_entry_by_name, LoadConfig};
use foundry_common::{evm, fs, retry::Retry};
use foundry_block_explorers::{
    Client,
};
use foundry_compilers::ConfigurableContractArtifact;
use futures::FutureExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, str::FromStr};

pub static OKLINK_URL: &str = "https://www.oklink.com/api/v5/explorer/contract/verify-source-code-plugin/";

/// The type that can verify a contract on `oklink`
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct OklinkVerificationProvider;

#[async_trait]
impl VerificationProvider for OklinkVerificationProvider {
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
                        .post(args.verifier.verifier_url.as_deref().unwrap_or(OKLINK_URL))
                        .header("Content-Type", "application/json")
                        .body(serde_json::to_string(&body)?)
                        .send()
                        .await?;

                    let status = response.status();
                    if !status.is_success() {
                        let error: serde_json::Value = response.json().await?;
                        eyre::bail!(
                            "Oklink verification request for address ({}) failed with status code {status}\nDetails: {error:#}",
                            args.address,
                        );
                    }

                    let text = response.text().await?;
                    Ok(Some(serde_json::from_str::<OklinkVerificationResponse>(&text)?))
                }
                .boxed()
            })
            .await?;

        self.process_oklink_response(resp.map(|r| r.result))
    }

    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| {
                async {
                    let url = Url::from_str(
                        args.verifier.verifier_url.as_deref().unwrap_or(OKLINK_URL),
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

                    Ok(Some(response.json::<Vec<OklinkResponseElement>>().await?))
                }
                .boxed()
            })
            .await?;

        self.process_oklink_response(resp)
    }
}

impl OklinkVerificationProvider {
    /// Configures the API request to the oklink API using the given [`VerifyArgs`].
    fn prepare_request(&self, args: &VerifyArgs) -> Result<OklinkVerifyRequest> {
        let mut config = args.try_load_config_emit_warnings()?;
        config.libraries.extend(args.libraries.clone());

        let project = config.project()?;

        if !config.cache {
            eyre::bail!("Cache is required for oklink verification.")
        }

        let cache = project.read_cache_file()?;
        let (path, entry) = get_cached_entry_by_name(&cache, &args.contract.name)?;



        // the metadata is included in the contract's artifact file
        let artifact_path = entry
            .find_artifact_path(&args.contract.name)
            .ok_or_else(|| eyre::eyre!("No artifact found for contract {}", args.contract.name))?;

        let artifact: ConfigurableContractArtifact = fs::read_json_file(artifact_path)?;
        let compiler_version;
        let optimization_used;
        let runs;

        let evm_version;
        let license_type;
        let mut library_name:Vec<String> = vec![];
        let mut library_address: Vec<String> = vec![];

        if let Some(metadata) = artifact.metadata {
            compiler_version = metadata.compiler.version.clone();
            let settings = metadata.settings;
            evm_version = match settings.evm_version {
                Some(version) => Some(version.as_str().to_string()),
                None => None,
            };
            optimization_used = match settings.optimizer.enabled {
                Some(enabled) => match enabled {
                    true => "1".to_string(),
                    false => "0".to_string(),
                },
                None => "0".to_string(),
            };
            runs = match settings.optimizer.runs {
                Some(runs) => Some(runs.to_string()),
                None => None,
            };
            let contract_path = args.contract.path.as_ref().unwrap();
            license_type = match metadata.sources.inner.get(contract_path) {
                Some(metadata_source) => metadata_source.license.clone(),
                None => None,
            };
            for (name, address) in settings.libraries.into_iter() {
                library_name.push(name.clone());
                library_address.push(address.clone());
            }

        } else {
            eyre::bail!(
                r#"No metadata found in artifact `{}` for contract {}.
Oklink requires contract metadata for verification.
metadata output can be enabled via `extra_output = ["metadata"]` in `foundry.toml`"#,
                artifact_path.display(),
                args.contract.name
            )
        }

        let contract_path = args.contract.path.clone().map_or(path, PathBuf::from);
        let source_code = fs::read_to_string(&contract_path)?;


        let req = OklinkVerifyRequest {
            sourceCode: source_code,
            contractaddress: args.address.clone().to_string(),
            // currently only single file supported
            codeformat: CodeFormat::SingleFile.as_str().to_string(),
            contractname: args.contract.name.clone(),
            compilerversion: compiler_version,
            optimizationUsed: optimization_used,
            runs: runs,
            constructorArguments: args.constructor_args.clone(),
            evmversion: evm_version,
            licenseType: license_type,
            libraryname: Some(library_name.join(",")),
            libraryaddress: Some(library_address.join(",")),

        };

        Ok(req)
    }

    fn process_oklink_response(
        &self,
        response: Option<Vec<OklinkResponseElement>>,
    ) -> Result<()> {
        let Some([response, ..]) = response.as_deref() else { return Ok(()) };
        match response.status.as_str() {
            "perfect" => {
                if let Some(ts) = &response.GUID {
                    println!("Contract source code already verified. Storage Timestamp: {ts}");
                } else {
                    println!("Contract successfully verified");
                }
            }
            "partial" => {
                println!("The recompiled contract partially matches the deployed version");
            }
            "false" => println!("Contract source code is not verified"),
            s => eyre::bail!("Unknown status from oklink. Status: {s:?}"),
        }
        Ok(())
    }
}


#[derive(Debug, Serialize)]
pub enum CodeFormat {
    SingleFile,
    JsonInput,
    Vyper,
}
impl CodeFormat {
    fn as_str(&self) -> &'static str {
        match self {
            CodeFormat::SingleFile => "solidity-single-file",
            CodeFormat::JsonInput => "solidity-standard-json-input",
            CodeFormat::Vyper => "Vyper",
        }
    }
}
#[derive(Debug, Serialize)]
pub struct OklinkVerifyRequest {
    sourceCode: String,
    contractaddress: String,
    codeformat: String,
    contractname: String,
    compilerversion: String,
    optimizationUsed:String,
    runs: Option<String>,
    constructorArguments: Option<String>,
    evmversion:Option<String>,
    licenseType:Option<String>,
    libraryname:Option<String>,
    libraryaddress:Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OklinkVerificationResponse {
    result: Vec<OklinkResponseElement>,
}

#[derive(Debug, Deserialize)]
pub struct OklinkResponseElement {
    status: String,
    message: String,
    #[serde(rename = "result")]
    GUID: Option<String>,
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
