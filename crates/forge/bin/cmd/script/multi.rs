use super::{
    receipts,
    sequence::{sig_to_file_name, ScriptSequence, SensitiveScriptSequence, DRY_RUN_DIR},
    verify::VerifyBundle,
    ScriptArgs,
};
use ethers_signers::LocalWallet;
use eyre::{ContextCompat, Report, Result, WrapErr};
use foundry_cli::utils::now;
use foundry_common::{fs, provider::ethers::get_http_provider};
use foundry_compilers::{artifacts::Libraries, ArtifactId};
use foundry_config::Config;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Holds the sequences of multiple chain deployments.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct MultiChainSequence {
    pub deployments: Vec<ScriptSequence>,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub sensitive_path: PathBuf,
    pub timestamp: u64,
}

/// Sensitive values from script sequences.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SensitiveMultiChainSequence {
    pub deployments: Vec<SensitiveScriptSequence>,
}

fn to_sensitive(sequence: &mut MultiChainSequence) -> SensitiveMultiChainSequence {
    SensitiveMultiChainSequence {
        deployments: sequence.deployments.iter_mut().map(|sequence| sequence.into()).collect(),
    }
}

impl Drop for MultiChainSequence {
    fn drop(&mut self) {
        self.deployments.iter_mut().for_each(|sequence| sequence.sort_receipts());
        self.save().expect("could not save multi deployment sequence");
    }
}

impl MultiChainSequence {
    pub fn new(
        deployments: Vec<ScriptSequence>,
        sig: &str,
        target: &ArtifactId,
        config: &Config,
        broadcasted: bool,
    ) -> Result<Self> {
        let (path, sensitive_path) = MultiChainSequence::get_paths(
            &config.broadcast,
            &config.cache_path,
            sig,
            target,
            broadcasted,
        )?;

        Ok(MultiChainSequence { deployments, path, sensitive_path, timestamp: now().as_secs() })
    }

    /// Gets paths in the formats
    /// ./broadcast/multi/contract_filename[-timestamp]/sig.json and
    /// ./cache/multi/contract_filename[-timestamp]/sig.json
    pub fn get_paths(
        broadcast: &Path,
        cache: &Path,
        sig: &str,
        target: &ArtifactId,
        broadcasted: bool,
    ) -> Result<(PathBuf, PathBuf)> {
        let mut broadcast = broadcast.to_path_buf();
        let mut cache = cache.to_path_buf();
        let mut common = PathBuf::new();

        common.push("multi");

        if !broadcasted {
            common.push(DRY_RUN_DIR);
        }

        let target_fname = target
            .source
            .file_name()
            .wrap_err_with(|| format!("No filename for {:?}", target.source))?
            .to_string_lossy();

        common.push(format!("{target_fname}-latest"));

        broadcast.push(common.clone());
        cache.push(common);

        fs::create_dir_all(&broadcast)?;
        fs::create_dir_all(&cache)?;

        let filename = format!("{}.json", sig_to_file_name(sig));

        broadcast.push(filename.clone());
        cache.push(filename);

        Ok((broadcast, cache))
    }

    /// Loads the sequences for the multi chain deployment.
    pub fn load(config: &Config, sig: &str, target: &ArtifactId) -> Result<Self> {
        let (path, sensitive_path) = MultiChainSequence::get_paths(
            &config.broadcast,
            &config.cache_path,
            sig,
            target,
            true,
        )?;
        let mut sequence: MultiChainSequence = foundry_compilers::utils::read_json_file(&path)
            .wrap_err("Multi-chain deployment not found.")?;
        let sensitive_sequence: SensitiveMultiChainSequence =
            foundry_compilers::utils::read_json_file(&sensitive_path)
                .wrap_err("Multi-chain deployment sensitive details not found.")?;

        sequence.deployments.iter_mut().enumerate().for_each(|(i, sequence)| {
            sequence.fill_sensitive(&sensitive_sequence.deployments[i]);
        });

        sequence.path = path;
        sequence.sensitive_path = sensitive_path;

        Ok(sequence)
    }

    /// Saves the transactions as file if it's a standalone deployment.
    pub fn save(&mut self) -> Result<()> {
        self.timestamp = now().as_secs();

        let sensitive_sequence: SensitiveMultiChainSequence = to_sensitive(self);

        // broadcast writes
        //../Contract-latest/run.json
        let mut writer = BufWriter::new(fs::create_file(&self.path)?);
        serde_json::to_writer_pretty(&mut writer, &self)?;
        writer.flush()?;

        //../Contract-[timestamp]/run.json
        let path = self.path.to_string_lossy();
        let file = PathBuf::from(&path.replace("-latest", &format!("-{}", self.timestamp)));
        fs::create_dir_all(file.parent().unwrap())?;
        fs::copy(&self.path, &file)?;

        // cache writes
        //../Contract-latest/run.json
        let mut writer = BufWriter::new(fs::create_file(&self.sensitive_path)?);
        serde_json::to_writer_pretty(&mut writer, &sensitive_sequence)?;
        writer.flush()?;

        //../Contract-[timestamp]/run.json
        let path = self.sensitive_path.to_string_lossy();
        let file = PathBuf::from(&path.replace("-latest", &format!("-{}", self.timestamp)));
        fs::create_dir_all(file.parent().unwrap())?;
        fs::copy(&self.sensitive_path, &file)?;

        println!("\nTransactions saved to: {}\n", self.path.display());
        println!("Sensitive details saved to: {}\n", self.sensitive_path.display());

        Ok(())
    }
}

impl ScriptArgs {
    /// Given a [`MultiChainSequence`] with multiple sequences of different chains, it executes them
    /// all in parallel. Supports `--resume` and `--verify`.
    pub async fn multi_chain_deployment(
        &self,
        mut deployments: MultiChainSequence,
        libraries: Libraries,
        config: &Config,
        script_wallets: Vec<LocalWallet>,
        verify: VerifyBundle,
    ) -> Result<()> {
        if !libraries.is_empty() {
            eyre::bail!("Libraries are currently not supported on multi deployment setups.");
        }

        if self.verify {
            for sequence in &deployments.deployments {
                sequence.verify_preflight_check(config, &verify)?;
            }
        }

        if self.resume {
            trace!(target: "script", "resuming multi chain deployment");

            let futs = deployments
                .deployments
                .iter_mut()
                .map(|sequence| async move {
                    let provider = Arc::new(get_http_provider(
                        sequence.typed_transactions().first().unwrap().0.clone(),
                    ));
                    receipts::wait_for_pending(provider, sequence).await
                })
                .collect::<Vec<_>>();

            let errors =
                join_all(futs).await.into_iter().filter(|res| res.is_err()).collect::<Vec<_>>();

            if !errors.is_empty() {
                return Err(eyre::eyre!("{errors:?}"));
            }
        }

        trace!(target: "script", "broadcasting multi chain deployments");

        let mut results: Vec<Result<(), Report>> = Vec::new();

        for sequence in deployments.deployments.iter_mut() {
            let result = match self
                .send_transactions(
                    sequence,
                    &sequence.typed_transactions().first().unwrap().0.clone(),
                    &script_wallets,
                )
                .await
            {
                Ok(_) if self.verify => sequence.verify_contracts(config, verify.clone()).await,
                Ok(_) => Ok(()),
                Err(err) => Err(err),
            };
            results.push(result);
        }

        let errors = results.into_iter().filter(|res| res.is_err()).collect::<Vec<_>>();

        if !errors.is_empty() {
            return Err(eyre::eyre!("{errors:?}"));
        }

        Ok(())
    }
}
