use crate::{
    etherscan::EtherscanVerificationProvider,
    sourcify::SourcifyVerificationProvider,
    verify::{VerifyArgs, VerifyCheckArgs},
};
use alloy_json_abi::JsonAbi;
use async_trait::async_trait;
use eyre::{OptionExt, Result};
use foundry_common::compile::ProjectCompiler;
use foundry_compilers::{
    artifacts::{output_selection::OutputSelection, Metadata, Source},
    compilers::{multi::MultiCompilerParsedSource, solc::SolcCompiler, CompilerSettings},
    solc::Solc,
    Graph, Project,
};
use foundry_config::Config;
use semver::Version;
use std::{fmt, path::PathBuf, str::FromStr};

/// Container with data required for contract verification.
#[derive(Debug, Clone)]
pub struct VerificationContext {
    pub config: Config,
    pub project: Project,
    pub target_path: PathBuf,
    pub target_name: String,
    pub compiler_version: Version,
}

impl VerificationContext {
    pub fn new(
        target_path: PathBuf,
        target_name: String,
        compiler_version: Version,
        config: Config,
    ) -> Result<Self> {
        let mut project = config.project()?;
        project.no_artifacts = true;

        let solc = Solc::find_or_install(&compiler_version)?;
        project.compiler.solc = Some(SolcCompiler::Specific(solc));

        Ok(Self { config, project, target_name, target_path, compiler_version })
    }

    /// Compiles target contract requesting only ABI and returns it.
    pub fn get_target_abi(&self) -> Result<JsonAbi> {
        let mut project = self.project.clone();
        project.settings.update_output_selection(|selection| {
            *selection = OutputSelection::common_output_selection(["abi".to_string()])
        });

        let output = ProjectCompiler::new()
            .quiet(true)
            .files([self.target_path.clone()])
            .compile(&project)?;

        let artifact = output
            .find(&self.target_path, &self.target_name)
            .ok_or_eyre("failed to find target artifact when compiling for abi")?;

        artifact.abi.clone().ok_or_eyre("target artifact does not have an ABI")
    }

    /// Compiles target file requesting only metadata and returns it.
    pub fn get_target_metadata(&self) -> Result<Metadata> {
        let mut project = self.project.clone();
        project.settings.update_output_selection(|selection| {
            *selection = OutputSelection::common_output_selection(["metadata".to_string()]);
        });

        let output = ProjectCompiler::new()
            .quiet(true)
            .files([self.target_path.clone()])
            .compile(&project)?;

        let artifact = output
            .find(&self.target_path, &self.target_name)
            .ok_or_eyre("failed to find target artifact when compiling for metadata")?;

        artifact.metadata.clone().ok_or_eyre("target artifact does not have an ABI")
    }

    /// Returns [Vec] containing imports of the target file.
    pub fn get_target_imports(&self) -> Result<Vec<PathBuf>> {
        let mut sources = self.project.paths.read_input_files()?;
        sources.insert(self.target_path.clone(), Source::read(&self.target_path)?);
        let graph =
            Graph::<MultiCompilerParsedSource>::resolve_sources(&self.project.paths, sources)?;

        Ok(graph.imports(&self.target_path).into_iter().cloned().collect())
    }
}

/// An abstraction for various verification providers such as etherscan, sourcify, blockscout
#[async_trait]
pub trait VerificationProvider {
    /// This should ensure the verify request can be prepared successfully.
    ///
    /// Caution: Implementers must ensure that this _never_ sends the actual verify request
    /// `[VerificationProvider::verify]`, instead this is supposed to evaluate whether the given
    /// [`VerifyArgs`] are valid to begin with. This should prevent situations where there's a
    /// contract deployment that's executed before the verify request and the subsequent verify task
    /// fails due to misconfiguration.
    async fn preflight_verify_check(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<()>;

    /// Sends the actual verify request for the targeted contract.
    async fn verify(&mut self, args: VerifyArgs, context: VerificationContext) -> Result<()>;

    /// Checks whether the contract is verified.
    async fn check(&self, args: VerifyCheckArgs) -> Result<()>;
}

impl FromStr for VerificationProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "e" | "etherscan" => Ok(Self::Etherscan),
            "s" | "sourcify" => Ok(Self::Sourcify),
            "b" | "blockscout" => Ok(Self::Blockscout),
            "o" | "oklink" => Ok(Self::Oklink),
            _ => Err(format!("Unknown provider: {s}")),
        }
    }
}

impl fmt::Display for VerificationProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Etherscan => {
                write!(f, "etherscan")?;
            }
            Self::Sourcify => {
                write!(f, "sourcify")?;
            }
            Self::Blockscout => {
                write!(f, "blockscout")?;
            }
            Self::Oklink => {
                write!(f, "oklink")?;
            }
        };
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum VerificationProviderType {
    #[default]
    Etherscan,
    Sourcify,
    Blockscout,
    Oklink,
}

impl VerificationProviderType {
    /// Returns the corresponding `VerificationProvider` for the key
    pub fn client(&self, key: &Option<String>) -> Result<Box<dyn VerificationProvider>> {
        match self {
            Self::Etherscan => {
                if key.as_ref().map_or(true, |key| key.is_empty()) {
                    eyre::bail!("ETHERSCAN_API_KEY must be set")
                }
                Ok(Box::<EtherscanVerificationProvider>::default())
            }
            Self::Sourcify => Ok(Box::<SourcifyVerificationProvider>::default()),
            Self::Blockscout => Ok(Box::<EtherscanVerificationProvider>::default()),
            Self::Oklink => Ok(Box::<EtherscanVerificationProvider>::default()),
        }
    }
}
