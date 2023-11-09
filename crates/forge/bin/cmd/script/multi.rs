use super::{
    receipts,
    sequence::{ScriptSequence, DRY_RUN_DIR},
    verify::VerifyBundle,
    ScriptArgs,
};
use ethers::signers::LocalWallet;
use eyre::{ContextCompat, Report, Result, WrapErr};
use foundry_cli::utils::now;
use foundry_common::{fs, get_http_provider};
use foundry_compilers::{artifacts::Libraries, ArtifactId};
use foundry_config::Config;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::log::trace;

/// Holds the sequences of multiple chain deployments.
#[derive(Deserialize, Serialize, Clone, Default)]
pub struct MultiChainSequence {
    pub deployments: Vec<ScriptSequence>,
    pub path: PathBuf,
    pub timestamp: u64,
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
        log_folder: &Path,
        broadcasted: bool,
    ) -> Result<Self> {
        let path =
            MultiChainSequence::get_path(&log_folder.join("multi"), sig, target, broadcasted)?;

        Ok(MultiChainSequence { deployments, path, timestamp: now().as_secs() })
    }

    /// Saves to ./broadcast/multi/contract_filename[-timestamp]/sig.json
    pub fn get_path(
        out: &Path,
        sig: &str,
        target: &ArtifactId,
        broadcasted: bool,
    ) -> Result<PathBuf> {
        let mut out = out.to_path_buf();

        if !broadcasted {
            out.push(DRY_RUN_DIR);
        }

        let target_fname = target
            .source
            .file_name()
            .wrap_err_with(|| format!("No filename for {:?}", target.source))?
            .to_string_lossy();
        out.push(format!("{target_fname}-latest"));

        fs::create_dir_all(&out)?;

        let filename = sig
            .split_once('(')
            .wrap_err_with(|| format!("Failed to compute file name: Signature {sig} is invalid."))?
            .0;
        out.push(format!("{filename}.json"));

        Ok(out)
    }

    /// Loads the sequences for the multi chain deployment.
    pub fn load(log_folder: &Path, sig: &str, target: &ArtifactId) -> Result<Self> {
        let path = MultiChainSequence::get_path(&log_folder.join("multi"), sig, target, true)?;
        foundry_compilers::utils::read_json_file(path).wrap_err("Multi-chain deployment not found.")
    }

    /// Saves the transactions as file if it's a standalone deployment.
    pub fn save(&mut self) -> Result<()> {
        self.timestamp = now().as_secs();

        //../Contract-latest/run.json
        let mut writer = BufWriter::new(fs::create_file(&self.path)?);
        serde_json::to_writer_pretty(&mut writer, &self)?;
        writer.flush()?;

        //../Contract-[timestamp]/run.json
        let path = self.path.to_string_lossy();
        let file = PathBuf::from(&path.replace("-latest", &format!("-{}", self.timestamp)));
        fs::create_dir_all(file.parent().unwrap())?;
        fs::copy(&self.path, &file)?;

        println!("\nTransactions saved to: {}\n", self.path.display());

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

        let mut results: Vec<Result<(), Report>> = Vec::new(); // Declare a Vec of Result<(), Report>

        for sequence in deployments.deployments.iter_mut() {
            let fork_url = &sequence.typed_transactions().first().unwrap().0.clone();
            println!("fork url is {fork_url}");
            let result = match self
                .send_transactions(
                    sequence,
                    &sequence.typed_transactions().first().unwrap().0.clone(),
                    &script_wallets,
                )
                .await
            {
                Ok(_) => {
                    if self.verify {
                        return sequence.verify_contracts(config, verify.clone()).await
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            };
            results.push(result);
        }

        let errors =
            results.into_iter().filter(|res| res.is_err()).collect::<Vec<_>>();

        if !errors.is_empty() {
            return Err(eyre::eyre!("{errors:?}"))
        }

        Ok(())
    }
}
