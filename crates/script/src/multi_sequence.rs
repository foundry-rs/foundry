use super::sequence::{sig_to_file_name, ScriptSequence, SensitiveScriptSequence, DRY_RUN_DIR};
use eyre::{ContextCompat, Result, WrapErr};
use foundry_cli::utils::now;
use foundry_common::fs;
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufWriter, Write},
    path::PathBuf,
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

impl SensitiveMultiChainSequence {
    fn from_multi_sequence(sequence: MultiChainSequence) -> Self {
        Self {
            deployments: sequence.deployments.into_iter().map(|sequence| sequence.into()).collect(),
        }
    }
}

impl MultiChainSequence {
    pub fn new(
        deployments: Vec<ScriptSequence>,
        sig: &str,
        target: &ArtifactId,
        config: &Config,
        dry_run: bool,
    ) -> Result<Self> {
        let (path, sensitive_path) = Self::get_paths(config, sig, target, dry_run)?;

        Ok(Self { deployments, path, sensitive_path, timestamp: now().as_secs() })
    }

    /// Gets paths in the formats
    /// ./broadcast/multi/contract_filename[-timestamp]/sig.json and
    /// ./cache/multi/contract_filename[-timestamp]/sig.json
    pub fn get_paths(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        dry_run: bool,
    ) -> Result<(PathBuf, PathBuf)> {
        let mut broadcast = config.broadcast.to_path_buf();
        let mut cache = config.cache_path.to_path_buf();
        let mut common = PathBuf::new();

        common.push("multi");

        if dry_run {
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
    pub fn load(config: &Config, sig: &str, target: &ArtifactId, dry_run: bool) -> Result<Self> {
        let (path, sensitive_path) = Self::get_paths(config, sig, target, dry_run)?;
        let mut sequence: Self = foundry_compilers::utils::read_json_file(&path)
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
    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        self.deployments.iter_mut().for_each(|sequence| sequence.sort_receipts());

        self.timestamp = now().as_secs();

        let sensitive_sequence = SensitiveMultiChainSequence::from_multi_sequence(self.clone());

        // broadcast writes
        //../Contract-latest/run.json
        let mut writer = BufWriter::new(fs::create_file(&self.path)?);
        serde_json::to_writer_pretty(&mut writer, &self)?;
        writer.flush()?;

        if save_ts {
            //../Contract-[timestamp]/run.json
            let path = self.path.to_string_lossy();
            let file = PathBuf::from(&path.replace("-latest", &format!("-{}", self.timestamp)));
            fs::create_dir_all(file.parent().unwrap())?;
            fs::copy(&self.path, &file)?;
        }

        // cache writes
        //../Contract-latest/run.json
        let mut writer = BufWriter::new(fs::create_file(&self.sensitive_path)?);
        serde_json::to_writer_pretty(&mut writer, &sensitive_sequence)?;
        writer.flush()?;

        if save_ts {
            //../Contract-[timestamp]/run.json
            let path = self.sensitive_path.to_string_lossy();
            let file = PathBuf::from(&path.replace("-latest", &format!("-{}", self.timestamp)));
            fs::create_dir_all(file.parent().unwrap())?;
            fs::copy(&self.sensitive_path, &file)?;
        }

        if !silent {
            println!("\nTransactions saved to: {}\n", self.path.display());
            println!("Sensitive details saved to: {}\n", self.sensitive_path.display());
        }

        Ok(())
    }
}
