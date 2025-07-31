use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use eyre::{eyre, Result, WrapErr};
use flate2::{write::GzEncoder, Compression};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write, path::Path, time::Duration};
use tar::Builder;

/// Git source information
#[derive(Debug, Clone, Serialize)]
pub struct GitSourceInfo {
    pub repository_url: String,
    pub commit_ref: String,
    pub project_path: String,
}

/// Archive source information
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveSourceInfo {
    pub content: String, // Base64 encoded bytes
    pub project_path: String,
}

/// Source mode enum for git or archive verification
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SourceMode {
    Git { git_source: GitSourceInfo },
    Archive { archive_source: ArchiveSourceInfo },
}

/// Compile settings for the contract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileSettings {
    pub sdk_version: String,
    pub features: Vec<String>,
    pub no_default_features: bool,
}

impl Default for CompileSettings {
    fn default() -> Self {
        Self { sdk_version: "v0.3.6-dev".to_string(), features: vec![], no_default_features: false }
    }
}

/// Verification request structure
#[derive(Debug, Serialize)]
pub struct VerificationRequest {
    pub contract_name: String,
    pub address_hash: String,
    #[serde(flatten)]
    pub source_mode: SourceMode,
    pub compile_settings: CompileSettings,
    pub abi: serde_json::Value,
}

impl VerificationRequest {
    /// Create new verification request with archive source
    pub async fn new_archive(
        contract_name: String,
        address_hash: String,
        contract_path: &Path,
        compile_settings: CompileSettings,
        abi: serde_json::Value,
    ) -> Result<Self> {
        let source_mode = ArchiveSourceBuilder::create(contract_path).await?;

        Ok(Self { contract_name, address_hash, source_mode, compile_settings, abi })
    }

    /// Create new verification request with git source (manual parameters)
    pub fn new_git(
        contract_name: String,
        address_hash: String,
        repository_url: String,
        commit_ref: String,
        project_path: String,
        compile_settings: CompileSettings,
        abi: serde_json::Value,
    ) -> Self {
        let git_source = GitSourceInfo { repository_url, commit_ref, project_path };

        Self {
            contract_name,
            address_hash,
            source_mode: SourceMode::Git { git_source },
            compile_settings,
            abi,
        }
    }
}

/// Response from the verification API
#[derive(Debug, Deserialize)]
pub struct VerificationResponse {
    pub message: Option<String>,
}

/// Response wrapper for error cases
#[derive(Debug, Deserialize)]
pub struct ApiErrorResponse {
    pub message: String,
    pub errors: Option<HashMap<String, Vec<String>>>,
}

/// Main Fluent verification client
pub struct FluentVerificationClient {
    base_url: String,
    http_client: Client,
}

impl FluentVerificationClient {
    /// Create a new verification client
    pub fn new(base_url: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent("fluent-verification-client/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self { base_url: base_url.trim_end_matches('/').to_string(), http_client }
    }

    /// Verify contract using prepared request
    pub async fn verify(&self, request: VerificationRequest) -> Result<()> {
        self.send_verification_request(request).await
    }

    /// Send the verification request to Blockscout API
    async fn send_verification_request(&self, request: VerificationRequest) -> Result<()> {
        let url = format!(
            "{}/v2/smart-contracts/{}/verification/via/fluent",
            self.base_url, request.address_hash
        );

        // Add delay to allow contract indexing
        println!("Waiting 15 seconds for contract to be indexed...");
        std::thread::sleep(Duration::from_secs(15));

        println!("Sending verification request to: {}", url);
        println!("Contract: {} at address: {}", request.contract_name, request.address_hash);

        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .send()
            .await
            .wrap_err("Failed to send HTTP request")?;

        let status = response.status();

        if status.is_success() {
            // Get response as text first to see what we actually received
            let response_text = response.text().await.wrap_err("Failed to read response")?;
            println!("Response: {}", response_text);

            Ok(())
        } else {
            let error_text = response.text().await.wrap_err("Failed to read error response")?;

            // Try to parse as structured error
            if let Ok(error_response) = serde_json::from_str::<ApiErrorResponse>(&error_text) {
                Err(eyre!("API error ({}): {}", status.as_u16(), error_response.message))
            } else {
                Err(eyre!("API error ({}): {}", status.as_u16(), error_text))
            }
        }
    }
}

/// Archive source helper
struct ArchiveSourceBuilder;

impl ArchiveSourceBuilder {
    async fn create(contract_path: &Path) -> Result<SourceMode> {
        let archive_content = Self::create_archive_from_path(contract_path).await?;
        Ok(SourceMode::Archive {
            archive_source: ArchiveSourceInfo {
                content: archive_content,
                project_path: ".".to_string(),
            },
        })
    }

    /// Create a Base64-encoded tar.gz archive from the contract path
    async fn create_archive_from_path(contract_path: &Path) -> Result<String> {
        if contract_path.is_file() {
            // Single file - create archive with the file and infer project structure
            let file_name = contract_path
                .file_name()
                .ok_or_else(|| eyre!("Invalid file name"))?
                .to_string_lossy();

            let content =
                fs::read_to_string(contract_path).wrap_err("Failed to read contract file")?;

            // For single files, check if it's in a project directory structure
            if let Some(parent) = contract_path.parent() {
                if parent.join("Cargo.toml").exists() {
                    // It's part of a Rust project, archive the whole project
                    return Self::create_tar_gz_archive(parent).await;
                }
            }

            // Create a minimal project structure for a single file
            let files = vec![(file_name.to_string(), content)];
            Self::create_tar_gz_from_files(&files).await
        } else if contract_path.is_dir() {
            // Directory - create tar.gz archive of the entire directory
            Self::create_tar_gz_archive(contract_path).await
        } else {
            Err(eyre!("Path is neither a file nor a directory"))
        }
    }

    /// Create a tar.gz archive from a directory
    async fn create_tar_gz_archive(dir_path: &Path) -> Result<String> {
        let files = Self::collect_contract_files(dir_path)?;
        Self::create_tar_gz_from_files(&files).await
    }

    /// Create a tar.gz archive from a list of (path, content) tuples
    async fn create_tar_gz_from_files(files: &[(String, String)]) -> Result<String> {
        let mut tar_data = Vec::new();

        // Create tar archive
        {
            let mut tar = Builder::new(&mut tar_data);

            for (path, content) in files {
                let mut header = tar::Header::new_gnu();
                header
                    .set_path(path)
                    .wrap_err_with(|| format!("Failed to set path for {}", path))?;
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();

                tar.append(&header, content.as_bytes())
                    .wrap_err_with(|| format!("Failed to append file {}", path))?;
            }

            tar.finish().wrap_err("Failed to finalize tar archive")?;
        }

        // Compress with gzip
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).wrap_err("Failed to write tar data to gzip encoder")?;
        let compressed_data = encoder.finish().wrap_err("Failed to finish gzip compression")?;

        // Encode to base64
        Ok(BASE64.encode(&compressed_data))
    }

    /// Collect all contract files from a directory (only .rs, .toml, .lock)
    fn collect_contract_files(dir: &Path) -> Result<Vec<(String, String)>> {
        let mut files = Vec::new();

        fn visit_dir(
            dir: &Path,
            base_path: &Path,
            files: &mut Vec<(String, String)>,
        ) -> Result<()> {
            for entry in fs::read_dir(dir)
                .wrap_err_with(|| format!("Failed to read directory: {}", dir.display()))?
            {
                let entry = entry.wrap_err("Failed to read directory entry")?;
                let path = entry.path();

                if path.is_file() {
                    // Include only specific file types for WASM contracts
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy();
                        if matches!(ext_str.as_ref(), "rs" | "toml" | "lock") {
                            let content = fs::read_to_string(&path).wrap_err_with(|| {
                                format!("Failed to read file: {}", path.display())
                            })?;
                            let relative_path = path
                                .strip_prefix(base_path)
                                .wrap_err("Failed to create relative path")?
                                .to_string_lossy()
                                .to_string();
                            files.push((relative_path, content));
                        }
                    }
                } else if path.is_dir() {
                    // Skip directories that shouldn't be included
                    if let Some(dir_name) = path.file_name() {
                        let dir_str = dir_name.to_string_lossy();
                        if !matches!(dir_str.as_ref(), "target" | ".git" | "node_modules")
                            && !dir_str.starts_with('.')
                        {
                            visit_dir(&path, base_path, files)?;
                        }
                    }
                }
            }
            Ok(())
        }

        visit_dir(dir, dir, &mut files)?;

        if files.is_empty() {
            return Err(eyre!("No contract files (.rs, .toml, .lock) found in directory"));
        }

        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_archive_source_creation() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("lib.rs");
        fs::write(&file_path, "fn main() {}").unwrap();

        let result = ArchiveSourceBuilder::create(&file_path).await;
        assert!(result.is_ok());

        if let SourceMode::Archive { archive_source } = result.unwrap() {
            assert!(!archive_source.content.is_empty());
            assert_eq!(archive_source.project_path, ".");
        } else {
            panic!("Expected Archive source mode");
        }
    }

    #[test]
    fn test_git_request_creation() {
        let request = VerificationRequest::new_git(
            "TestContract".to_string(),
            "0x1234567890abcdef".to_string(),
            "https://github.com/test/repo.git".to_string(),
            "main".to_string(),
            ".".to_string(),
            CompileSettings::default(),
            json!([]),
        );

        if let SourceMode::Git { git_source } = request.source_mode {
            assert_eq!(git_source.repository_url, "https://github.com/test/repo.git");
            assert_eq!(git_source.commit_ref, "main");
            assert_eq!(git_source.project_path, ".");
        } else {
            panic!("Expected Git source mode");
        }
    }

    #[test]
    fn test_client_creation() {
        let client = FluentVerificationClient::new("https://example.com/".to_string());
        assert_eq!(client.base_url, "https://example.com");
    }

    #[test]
    fn test_serialization() {
        let request = VerificationRequest::new_git(
            "TestContract".to_string(),
            "0x1234".to_string(),
            "https://github.com/test/repo.git".to_string(),
            "main".to_string(),
            ".".to_string(),
            CompileSettings::default(),
            json!([]),
        );

        let serialized = serde_json::to_string(&request).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        // Check that git_source is flattened correctly
        assert!(parsed.get("git_source").is_some());
        assert!(parsed.get("contract_name").is_some());
        assert!(parsed.get("address_hash").is_some());
    }
}
