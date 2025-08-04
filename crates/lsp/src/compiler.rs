use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;
use tower_lsp::async_trait;

pub struct ForgeCompiler;

#[async_trait]
pub trait Compiler: Send + Sync {
    async fn lint(&self, file: &str) -> Result<serde_json::Value, CompilerError>;
    async fn build(&self, file: &str) -> Result<serde_json::Value, CompilerError>;
}

#[derive(Error, Debug)]
pub enum CompilerError {
    #[error("Invalid file URL")]
    InvalidUrl,
    #[error("Failed to run command: {0}")]
    CommandError(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Empty output from compiler")]
    EmptyOutput,
    #[error("ReadError")]
    ReadError,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeSourceLocation {
    file: String,
    start: i32, // Changed to i32 to handle -1 values
    end: i32,   // Changed to i32 to handle -1 values
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeCompileError {
    #[serde(rename = "sourceLocation")]
    source_location: ForgeSourceLocation,
    #[serde(rename = "type")]
    error_type: String,
    component: String,
    severity: String,
    #[serde(rename = "errorCode")]
    error_code: String,
    message: String,
    #[serde(rename = "formattedMessage")]
    formatted_message: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeCompileOutput {
    errors: Option<Vec<ForgeCompileError>>,
    sources: serde_json::Value,
    contracts: serde_json::Value,
    build_infos: Vec<serde_json::Value>,
}

#[async_trait]
impl Compiler for ForgeCompiler {
    async fn lint(&self, file_path: &str) -> Result<serde_json::Value, CompilerError> {
        let output =
            Command::new("forge").arg("lint").arg(file_path).arg("--json").output().await?;

        let stderr_str = String::from_utf8_lossy(&output.stderr);

        // Parse JSON output line by line
        let mut diagnostics = Vec::new();
        for line in stderr_str.lines() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value) => diagnostics.push(value),
                Err(_e) => {
                    continue;
                }
            }
        }

        Ok(serde_json::Value::Array(diagnostics))
    }

    async fn build(&self, file_path: &str) -> Result<serde_json::Value, CompilerError> {
        let output = Command::new("forge")
            .arg("build")
            .arg(file_path)
            .arg("--json")
            .arg("--no-cache")
            .arg("--ast")
            .output()
            .await?;

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout_str)?;

        Ok(parsed)
    }
}
