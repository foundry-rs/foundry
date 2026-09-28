use crate::{
    multi_sequence::MultiChainSequence,
    recovery::{RecoveryLock, RecoveryStore, SignedPayload},
};
use alloy_network::Network;
use alloy_primitives::{B256, Bytes};
use eyre::{Result, bail};
use forge_script_sequence::{ScriptSequence, TransactionWithMetadata};
use foundry_cli::utils::Git;
use foundry_common::{FoundryTransactionBuilder, fmt::UIfmt};
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Error, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "sequence",
    rename_all = "camelCase",
    bound(
        serialize = "N::TransactionRequest: Serialize, N::TxEnvelope: Serialize",
        deserialize = "N::TransactionRequest: for<'de2> Deserialize<'de2>, N::TxEnvelope: for<'de2> Deserialize<'de2>"
    )
)]
pub(crate) enum SequenceData<N: Network> {
    Single(ScriptSequence<N>),
    Multi(MultiChainSequence<N>),
}

impl<N: Network> SequenceData<N>
where
    N::TxEnvelope: for<'d> Deserialize<'d> + Serialize,
    N::TransactionRequest: for<'d> Deserialize<'d> + Serialize,
{
    fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        match self {
            Self::Single(sequence) => sequence.save(silent, save_ts),
            Self::Multi(sequence) => sequence.save(silent, save_ts),
        }
    }

    pub fn sequences(&self) -> &[ScriptSequence<N>] {
        match self {
            Self::Single(sequence) => std::slice::from_ref(sequence),
            Self::Multi(sequence) => &sequence.deployments,
        }
    }

    pub fn sequences_mut(&mut self) -> &mut [ScriptSequence<N>] {
        match self {
            Self::Single(sequence) => std::slice::from_mut(sequence),
            Self::Multi(sequence) => &mut sequence.deployments,
        }
    }

    pub const fn is_multi(&self) -> bool {
        matches!(self, Self::Multi(_))
    }

    pub fn paths(&self) -> (std::path::PathBuf, std::path::PathBuf) {
        match self {
            Self::Single(sequence) => sequence.paths.clone().expect("sequence paths not set"),
            Self::Multi(sequence) => (sequence.path.clone(), sequence.sensitive_path.clone()),
        }
    }

    pub(crate) fn set_paths(&mut self, paths: (PathBuf, PathBuf)) {
        match self {
            Self::Single(sequence) => sequence.paths = Some(paths),
            Self::Multi(sequence) => {
                sequence.path = paths.0;
                sequence.sensitive_path = paths.1;
            }
        }
    }

    pub(crate) fn has_recovery_generation(&self) -> bool {
        self.sequences().iter().any(|sequence| sequence.recovery_generation.is_some())
    }

    pub(crate) fn set_recovery_generation(&mut self, generation: B256) {
        for sequence in self.sequences_mut() {
            sequence.recovery_generation = Some(generation);
        }
    }

    pub(crate) fn publish(&self, paths: &(PathBuf, PathBuf)) -> Result<()> {
        let mut export = self.clone();
        export.set_paths(paths.clone());
        export.save(true, false)
    }

    fn save_timestamped(&self, paths: &(PathBuf, PathBuf)) -> Result<()> {
        match self {
            Self::Single(sequence) => {
                let filename = format!("run-{}.json", sequence.timestamp);
                std::fs::copy(&paths.0, paths.0.with_file_name(&filename))?;
                std::fs::copy(&paths.1, paths.1.with_file_name(filename))?;
            }
            Self::Multi(sequence) => {
                let timestamp = sequence.timestamp;
                for path in [&paths.0, &paths.1] {
                    let timestamped = PathBuf::from(
                        path.to_string_lossy().replace("-latest", &format!("-{timestamp}")),
                    );
                    std::fs::create_dir_all(timestamped.parent().unwrap())?;
                    std::fs::copy(path, timestamped)?;
                }
            }
        }
        Ok(())
    }
}

pub struct ScriptSequenceKind<N: Network>
where
    N::TxEnvelope: for<'d> Deserialize<'d> + Serialize,
    N::TransactionRequest: for<'d> Deserialize<'d> + Serialize,
{
    recovery: RecoveryStore<N>,
}

impl<N: Network> ScriptSequenceKind<N>
where
    N::TxEnvelope: for<'d> Deserialize<'d> + Serialize,
    N::TransactionRequest: for<'d> Deserialize<'d> + Serialize,
{
    pub fn new_single(sequence: ScriptSequence<N>, batch: bool) -> Result<Self> {
        Self::create(SequenceData::Single(sequence), batch)
    }

    pub fn new_multi(sequence: MultiChainSequence<N>, batch: bool) -> Result<Self> {
        Self::create(SequenceData::Multi(sequence), batch)
    }

    pub fn load_single(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain: u64,
        dry_run: bool,
        batch: bool,
    ) -> Result<Self>
    where
        N::TxEnvelope: alloy_consensus::transaction::SignerRecoverable,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let paths = ScriptSequence::<N>::get_paths(config, sig, target, chain, dry_run)?;
        let lock = RecoveryLock::acquire(&paths)?;
        let recovery = if let Some(recovery) = RecoveryStore::load(&paths, batch, lock)? {
            recovery
        } else {
            let lock = RecoveryLock::acquire(&paths)?;
            let data =
                SequenceData::Single(ScriptSequence::load(config, sig, target, chain, dry_run)?);
            if data.has_recovery_generation() {
                bail!("recovery exports reference a missing authoritative snapshot");
            }
            RecoveryStore::import(data, batch, lock)?
        };
        let sequence = Self { recovery };
        sequence.recovery.data().publish(&paths)?;
        Ok(sequence)
    }

    pub fn load_multi(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        dry_run: bool,
        batch: bool,
    ) -> Result<Self>
    where
        N::TxEnvelope: alloy_consensus::transaction::SignerRecoverable,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let paths = MultiChainSequence::<N>::get_paths(config, sig, target, dry_run)?;
        let lock = RecoveryLock::acquire(&paths)?;
        let recovery = if let Some(recovery) = RecoveryStore::load(&paths, batch, lock)? {
            recovery
        } else {
            let lock = RecoveryLock::acquire(&paths)?;
            let data = SequenceData::Multi(MultiChainSequence::load(config, sig, target, dry_run)?);
            if data.has_recovery_generation() {
                bail!("recovery exports reference a missing authoritative snapshot");
            }
            RecoveryStore::import(data, batch, lock)?
        };
        let sequence = Self { recovery };
        sequence.recovery.data().publish(&paths)?;
        Ok(sequence)
    }

    fn create(data: SequenceData<N>, batch: bool) -> Result<Self> {
        let paths = data.paths();
        let recovery = RecoveryStore::create(data, batch)?;
        let sequence = Self { recovery };
        sequence.recovery.data().publish(&paths)?;
        Ok(sequence)
    }

    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        self.recovery.save()?;
        let paths = self.recovery.data().paths();
        self.recovery.data().publish(&paths)?;
        if save_ts {
            self.recovery.data().save_timestamped(&paths)?;
        }
        if !silent {
            if foundry_common::shell::is_json() {
                sh_println!(
                    "{}",
                    serde_json::json!({
                        "status": "success",
                        "transactions": paths.0.display().to_string(),
                        "sensitive": paths.1.display().to_string(),
                    })
                )?;
            } else {
                sh_println!("\nTransactions saved to: {}\n", paths.0.display())?;
                sh_println!("Sensitive values saved to: {}\n", paths.1.display())?;
            }
        }
        Ok(())
    }

    pub fn sequences(&self) -> &[ScriptSequence<N>] {
        self.recovery.data().sequences()
    }

    pub fn sequences_mut(&mut self) -> &mut [ScriptSequence<N>] {
        self.recovery.data_mut().sequences_mut()
    }

    pub(crate) fn signed_payload(&self, sequence: usize, index: usize) -> Option<&SignedPayload> {
        self.recovery.signed_payload(sequence, index)
    }

    pub(crate) fn submission_hashes(&self, sequence: usize) -> Vec<B256> {
        self.recovery.submission_hashes(sequence)
    }

    pub(crate) fn persist_signed_payload(
        &mut self,
        sequence: usize,
        index: usize,
        payload: Bytes,
    ) -> Result<B256>
    where
        N::TxEnvelope: alloy_consensus::transaction::SignerRecoverable,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        self.recovery.persist_signed_payload(sequence, index, payload)
    }

    pub const fn is_multi(&self) -> bool {
        self.recovery.data().is_multi()
    }

    /// Atomically promotes a dry-run sequence to the broadcast paths.
    pub fn promote_to_broadcasted(
        &mut self,
        config: &Config,
        sig: &str,
        target: &ArtifactId,
    ) -> Result<()> {
        let paths = match self.recovery.data() {
            SequenceData::Single(sequence) => {
                ScriptSequence::<N>::get_paths(config, sig, target, sequence.chain, false)?
            }
            SequenceData::Multi(_) => {
                MultiChainSequence::<N>::get_paths(config, sig, target, false)?
            }
        };
        let relocation = self.recovery.prepare_relocation(&paths)?;
        self.recovery.commit_relocation(relocation)?;
        self.recovery.data_mut().set_paths(paths.clone());
        self.recovery.save()?;
        self.recovery.data().publish(&paths)?;
        self.recovery.data().save_timestamped(&paths)?;
        Ok(())
    }

    pub fn show_transactions(&self) -> Result<()>
    where
        N::TxEnvelope: UIfmt,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        for sequence in self.sequences() {
            if !sequence.transactions.is_empty() {
                sh_println!("\nChain {}\n", sequence.chain)?;

                for (i, tx) in sequence.transactions.iter().enumerate() {
                    sh_print!("{}", format_transaction(i + 1, tx)?)?;
                }
            }
        }

        Ok(())
    }
}

impl<N: Network> Drop for ScriptSequenceKind<N>
where
    N::TxEnvelope: for<'d> Deserialize<'d> + Serialize,
    N::TransactionRequest: for<'d> Deserialize<'d> + Serialize,
{
    fn drop(&mut self) {
        if let Err(err) = self.save(false, true) {
            error!(?err, "could not save deployment sequence");
        }
    }
}

/// Format transaction details for display
fn format_transaction<N: Network>(
    index: usize,
    tx: &TransactionWithMetadata<N>,
) -> Result<String, Error>
where
    N::TxEnvelope: UIfmt,
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    let mut output = String::new();
    writeln!(output, "### Transaction {index} ###")?;
    writeln!(output, "{}", tx.tx().pretty())?;

    // Show contract name and address if available
    if !tx.call_kind.is_any_create()
        && let (Some(name), Some(addr)) = (&tx.contract_name, &tx.contract_address)
    {
        writeln!(output, "contract: {name}({addr})")?;
    }

    // Show decoded function if available
    if let (Some(func), Some(args)) = (&tx.display_function, &tx.arguments) {
        if args.is_empty() {
            writeln!(output, "data (decoded): {func}()")?;
        } else {
            writeln!(output, "data (decoded): {func}(")?;
            for (i, arg) in args.iter().enumerate() {
                writeln!(&mut output, "  {}{}", arg, if i + 1 < args.len() { "," } else { "" })?;
            }
            writeln!(output, ")")?;
        }
    }

    writeln!(output)?;
    Ok(output)
}

/// Returns the commit hash of the project if it exists
pub fn get_commit_hash(root: &Path) -> Option<String> {
    Git::new(root).commit_hash(true, "HEAD").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::Ethereum;
    use foundry_common::TransactionMaybeSigned;

    #[test]
    fn publish_keeps_source_paths() {
        let dir = tempfile::tempdir().unwrap();
        let source_paths = (dir.path().join("dry-run.json"), dir.path().join("dry-run-cache.json"));
        let mut sequence =
            ScriptSequence::<Ethereum> { paths: Some(source_paths.clone()), ..Default::default() };
        sequence.transactions.push_back(TransactionWithMetadata::from_tx_request(
            TransactionMaybeSigned::Unsigned(Default::default()),
        ));
        let data = SequenceData::Single(sequence);

        let public = dir.path().join("broadcast.json");
        let sensitive = dir.path().join("broadcast-cache.json");
        let paths = (public, sensitive.clone());
        data.publish(&paths).unwrap();

        assert_eq!(data.paths(), source_paths);
        let value: serde_json::Value = foundry_common::fs::read_json_file(&sensitive).unwrap();
        assert_eq!(value["transactions"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn any_deployment_generation_blocks_legacy_import() {
        let mut data = SequenceData::<Ethereum>::Multi(MultiChainSequence {
            deployments: vec![ScriptSequence::default(), ScriptSequence::default()],
            path: PathBuf::new(),
            sensitive_path: PathBuf::new(),
            timestamp: 0,
        });
        data.sequences_mut()[1].recovery_generation = Some(B256::repeat_byte(0x11));

        assert!(data.has_recovery_generation());
    }
}
