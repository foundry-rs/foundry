use super::{
    receipts,
    sequence::{ScriptSequence, DRY_RUN_DIR},
    verify::VerifyBundle,
    ScriptArgs,
};
use ethers::{
    prelude::{artifacts::Libraries, ArtifactId},
    signers::LocalWallet,
};
use eyre::{ContextCompat, WrapErr};
use foundry_common::{fs, get_http_provider};
use foundry_config::Config;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::{
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
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
        self.deployments.iter_mut().for_each(|sequence| {
            sequence.sort_receipts();
        });

        self.save().expect("not able to save multi deployment sequence");
    }
}

impl MultiChainSequence {
    pub fn new(
        deployments: Vec<ScriptSequence>,
        sig: &str,
        target: &ArtifactId,
        log_folder: &Path,
        broadcasted: bool,
    ) -> eyre::Result<Self> {
        let path =
            MultiChainSequence::get_path(&log_folder.join("multi"), sig, target, broadcasted)?;

        Ok(MultiChainSequence {
            deployments,
            path,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Wrong system time.")
                .as_secs(),
        })
    }

    /// Saves to ./broadcast/multi/contract_filename[-timestamp]/sig.json
    pub fn get_path(
        out: &Path,
        sig: &str,
        target: &ArtifactId,
        broadcasted: bool,
    ) -> eyre::Result<PathBuf> {
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
            .0
            .to_string();
        out.push(format!("{filename}.json"));

        Ok(out)
    }

    /// Loads the sequences for the multi chain deployment.
    pub fn load(log_folder: &Path, sig: &str, target: &ArtifactId) -> eyre::Result<Self> {
        let path = MultiChainSequence::get_path(&log_folder.join("multi"), sig, target, true)?;
        ethers::solc::utils::read_json_file(path).wrap_err("Multi-chain deployment not found.")
    }

    /// Saves the transactions as file if it's a standalone deployment.
    pub fn save(&mut self) -> eyre::Result<()> {
        self.timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let path = self.path.to_string_lossy();

        //../Contract-latest/run.json
        serde_json::to_writer_pretty(BufWriter::new(fs::create_file(&self.path)?), &self)?;

        //../Contract-[timestamp]/run.json
        let file = PathBuf::from(&path.replace("-latest", &format!("-{}", self.timestamp)));

        fs::create_dir_all(file.parent().expect("to have a file."))?;

        serde_json::to_writer_pretty(BufWriter::new(fs::create_file(file)?), &self)?;

        println!("\nTransactions saved to: {path}\n");

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
    ) -> eyre::Result<()> {
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
                        sequence.typed_transactions().first().unwrap().0.rpc.clone(),
                    ));
                    receipts::wait_for_pending(provider, sequence).await
                })
                .collect::<Vec<_>>();

            let errors =
                join_all(futs).await.into_iter().filter(|res| res.is_err()).collect::<Vec<_>>();

            if !errors.is_empty() {
                return Err(eyre::eyre!("{errors:?}"))
            }
        }

        trace!(target: "script", "broadcasting multi chain deployments");

        let futs = deployments
            .deployments
            .iter_mut()
            .map(|sequence| async {
                match self
                    .send_transactions(
                        sequence,
                        &sequence.typed_transactions().first().unwrap().0.rpc.clone(),
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
                }
            })
            .collect::<Vec<_>>();

        let errors =
            join_all(futs).await.into_iter().filter(|res| res.is_err()).collect::<Vec<_>>();

        if !errors.is_empty() {
            return Err(eyre::eyre!("{errors:?}"))
        }

        Ok(())
    }
}
