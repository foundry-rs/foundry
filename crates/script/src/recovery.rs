use crate::sequence::SequenceData;
use alloy_network::Network;
use alloy_primitives::{B256, keccak256};
use eyre::{ContextCompat, Result, WrapErr, bail};
use forge_script_sequence::TransactionWithMetadata;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const RECOVERY_VERSION: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "N::TransactionRequest: Serialize, N::TxEnvelope: Serialize",
    deserialize = "N::TransactionRequest: for<'de2> Deserialize<'de2>, N::TxEnvelope: for<'de2> Deserialize<'de2>"
))]
struct RecoveryPlan<N: Network> {
    version: u32,
    generation: B256,
    multi: bool,
    batch: bool,
    deployments: Vec<RecoveryDeployment>,
    data: SequenceData<N>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RecoveryDeployment {
    chain: u64,
    batch_id: Option<u32>,
    operations: Vec<RecoveryOperation>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RecoveryOperation {
    id: OperationId,
    fingerprint: B256,
    rpc: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct OperationId {
    sequence: u32,
    index: u32,
}

/// Owns the authoritative recovery snapshot and its process-lifetime writer lock.
pub(crate) struct RecoveryStore<N: Network> {
    path: PathBuf,
    _lock: RecoveryLock,
    plan: RecoveryPlan<N>,
}

pub(crate) struct RecoveryLock {
    path: PathBuf,
    _files: Vec<File>,
}

pub(crate) struct RecoveryRelocation {
    lock: RecoveryLock,
}

impl<N: Network> RecoveryStore<N>
where
    N::TransactionRequest: for<'de> Deserialize<'de> + Serialize,
    N::TxEnvelope: for<'de> Deserialize<'de> + Serialize,
{
    pub(crate) const fn data(&self) -> &SequenceData<N> {
        &self.plan.data
    }

    pub(crate) const fn data_mut(&mut self) -> &mut SequenceData<N> {
        &mut self.plan.data
    }

    pub(crate) fn create(mut data: SequenceData<N>, batch: bool) -> Result<Self> {
        let paths = data.paths();
        let lock = RecoveryLock::acquire(&paths)?;
        let pending = pending_path(&lock.path);
        if pending.exists() {
            bail!(
                "uncommitted recovery snapshot `{}` already exists; resume it before starting a new execution",
                pending.display()
            );
        }
        let generation = B256::random();
        data.set_recovery_generation(generation);
        let plan = RecoveryPlan::new(data, batch, generation)?;
        write_snapshot(&lock.path, &plan)?;
        Self::finish_open(lock, plan)
    }

    pub(crate) fn load(
        paths: &(PathBuf, PathBuf),
        batch: bool,
        lock: RecoveryLock,
    ) -> Result<Option<Self>> {
        let pending = pending_path(&lock.path);
        let (mut plan, recover_pending) = if pending.exists() {
            (load_plan(&pending)?, true)
        } else if lock.path.exists() {
            (load_plan(&lock.path)?, false)
        } else {
            return Ok(None);
        };
        plan.restore_sensitive();
        plan.data.set_paths(paths.clone());
        plan.validate(batch)?;
        if recover_pending {
            commit_pending_plan(&pending, &lock.path)?;
        }
        Self::finish_open(lock, plan).map(Some)
    }

    pub(crate) fn import(
        mut data: SequenceData<N>,
        batch: bool,
        lock: RecoveryLock,
    ) -> Result<Self> {
        let generation = B256::random();
        data.set_recovery_generation(generation);
        let plan = RecoveryPlan::new(data, batch, generation)?;
        write_snapshot(&lock.path, &plan)?;
        Self::finish_open(lock, plan)
    }

    fn finish_open(lock: RecoveryLock, plan: RecoveryPlan<N>) -> Result<Self> {
        #[cfg(unix)]
        for path in [&lock.path, &pending_path(&lock.path)] {
            if path.exists() {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        Ok(Self { path: lock.path.clone(), _lock: lock, plan })
    }

    pub(crate) fn save(&self) -> Result<()> {
        self.plan.validate(self.plan.batch)?;
        write_snapshot(&self.path, &self.plan)
    }

    pub(crate) fn prepare_relocation(
        &self,
        paths: &(PathBuf, PathBuf),
    ) -> Result<RecoveryRelocation> {
        let lock = RecoveryLock::acquire(paths)?;
        let path = &lock.path;
        if paths.0.exists() {
            bail!(
                "broadcast progress `{}` already exists; refusing to replace it",
                paths.0.display()
            );
        }
        if path.exists() {
            let existing: RecoveryPlan<N> = foundry_common::fs::read_json_file(path)
                .wrap_err_with(|| format!("recovery snapshot `{}` is corrupt", path.display()))?;
            existing.validate(self.plan.batch)?;
            self.plan.validate(self.plan.batch)?;
            if serde_json::to_value(existing.deployments)?
                != serde_json::to_value(&self.plan.deployments)?
            {
                bail!("destination recovery snapshot does not match the script operations");
            }
        }
        write_snapshot(path, &self.plan)?;
        Ok(RecoveryRelocation { lock })
    }

    pub(crate) fn commit_relocation(&mut self, relocation: RecoveryRelocation) -> Result<()> {
        let RecoveryRelocation { lock } = relocation;
        let path = &lock.path;
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        self.path.clone_from(path);
        self._lock = lock;
        Ok(())
    }
}

impl RecoveryLock {
    pub(crate) fn acquire(paths: &(PathBuf, PathBuf)) -> Result<Self> {
        let path = recovery_path_from_sensitive(&paths.1)?;
        let mut lock_paths =
            vec![path.with_extension("lock"), paths.0.with_extension("recovery.lock")];
        lock_paths.sort();
        lock_paths.dedup();
        let mut files = Vec::with_capacity(lock_paths.len());
        for lock_path in lock_paths {
            if let Some(parent) = lock_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            #[cfg(unix)]
            options.mode(0o600);
            let file = options.open(&lock_path)?;
            #[cfg(unix)]
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            file.try_lock().wrap_err_with(|| {
                format!("another process is already modifying recovery state `{}`", path.display())
            })?;
            files.push(file);
        }
        Ok(Self { path, _files: files })
    }
}

impl<N: Network> RecoveryPlan<N>
where
    N::TransactionRequest: Serialize,
    N::TxEnvelope: for<'de> Deserialize<'de> + Serialize,
{
    fn new(data: SequenceData<N>, batch: bool, generation: B256) -> Result<Self> {
        let deployments = data
            .sequences()
            .iter()
            .enumerate()
            .map(|(sequence, deployment)| {
                Ok(RecoveryDeployment {
                    chain: deployment.chain,
                    batch_id: batch.then(|| u32::try_from(sequence).expect("too many sequences")),
                    operations: deployment
                        .transactions
                        .iter()
                        .enumerate()
                        .map(|(index, transaction)| {
                            Ok(RecoveryOperation {
                                id: OperationId {
                                    sequence: u32::try_from(sequence).expect("too many sequences"),
                                    index: u32::try_from(index).expect("too many operations"),
                                },
                                fingerprint: operation_fingerprint(transaction)?,
                                rpc: transaction.rpc.clone(),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            version: RECOVERY_VERSION,
            generation,
            multi: data.is_multi(),
            batch,
            deployments,
            data,
        })
    }

    fn validate(&self, batch: bool) -> Result<()> {
        if self.version != RECOVERY_VERSION {
            bail!(
                "unsupported recovery snapshot version {}; expected version {}",
                self.version,
                RECOVERY_VERSION
            );
        }
        if self.batch != batch || self.multi != self.data.is_multi() {
            bail!("recovery snapshot does not match the requested script mode");
        }
        if self
            .data
            .sequences()
            .iter()
            .any(|sequence| sequence.recovery_generation != Some(self.generation))
        {
            bail!("recovery snapshot contains inconsistent generations");
        }
        let expected = Self::new(self.data.clone(), self.batch, self.generation)?;
        if serde_json::to_value(&self.deployments)? != serde_json::to_value(expected.deployments)? {
            bail!("recovery snapshot does not match the script operations; refusing to resume");
        }
        Ok(())
    }

    fn restore_sensitive(&mut self) {
        for (deployment, data) in self.deployments.iter().zip(self.data.sequences_mut().iter_mut())
        {
            for (operation, transaction) in
                deployment.operations.iter().zip(data.transactions.iter_mut())
            {
                transaction.rpc.clone_from(&operation.rpc);
            }
        }
    }
}

fn operation_fingerprint<N: Network>(transaction: &TransactionWithMetadata<N>) -> Result<B256>
where
    N::TransactionRequest: for<'de> Deserialize<'de> + Serialize,
    N::TxEnvelope: for<'de> Deserialize<'de> + Serialize,
{
    let mut transaction = transaction.clone();
    transaction.hash = None;
    transaction.rpc.clear();
    let transaction =
        serde_json::from_value::<TransactionWithMetadata<N>>(serde_json::to_value(transaction)?)?;
    Ok(keccak256(serde_json::to_vec(&canonicalize(serde_json::to_value(transaction)?))?))
}

fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize).collect())
        }
        serde_json::Value::Object(values) => {
            let mut values = values.into_iter().collect::<Vec<_>>();
            values.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
            serde_json::Value::Object(
                values.into_iter().map(|(key, value)| (key, canonicalize(value))).collect(),
            )
        }
        value => value,
    }
}

fn load_plan<N: Network>(path: &Path) -> Result<RecoveryPlan<N>>
where
    N::TransactionRequest: for<'de> Deserialize<'de>,
    N::TxEnvelope: for<'de> Deserialize<'de>,
{
    foundry_common::fs::read_json_file(path)
        .wrap_err_with(|| format!("recovery snapshot `{}` is corrupt", path.display()))
}

fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("pending")
}

pub(crate) fn recovery_exists(paths: &(PathBuf, PathBuf)) -> Result<bool> {
    let path = recovery_path_from_sensitive(&paths.1)?;
    Ok(path.exists() || pending_path(&path).exists())
}

fn commit_pending_plan(pending: &Path, path: &Path) -> Result<()> {
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(pending, path)?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    #[cfg(unix)]
    File::open(path.parent().context("recovery plan has no parent directory")?)?.sync_all()?;
    Ok(())
}

fn recovery_path_from_sensitive(sensitive_path: &Path) -> Result<PathBuf> {
    let filename = sensitive_path.file_name().context("sensitive cache path has no filename")?;
    Ok(sensitive_path.with_file_name(format!("{}.recovery.json", filename.to_string_lossy())))
}

fn write_snapshot<N: Network>(path: &Path, plan: &RecoveryPlan<N>) -> Result<()>
where
    N::TransactionRequest: Serialize,
    N::TxEnvelope: Serialize,
{
    let pending = pending_path(path);
    write_plan(&pending, plan)?;
    commit_pending_plan(&pending, path)
}

fn write_plan<N: Network>(path: &Path, plan: &RecoveryPlan<N>) -> Result<()>
where
    N::TransactionRequest: Serialize,
    N::TxEnvelope: Serialize,
{
    let parent = path.parent().context("recovery plan has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    tmp.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    {
        let mut writer = BufWriter::new(tmp.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, plan)?;
        writer.flush()?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::Ethereum;
    use foundry_common::TransactionMaybeSigned;

    fn sequence(dir: &Path) -> SequenceData<Ethereum> {
        let mut transaction = TransactionWithMetadata::from_tx_request(
            TransactionMaybeSigned::Unsigned(Default::default()),
        );
        transaction.rpc = "http://localhost:8545".to_string();
        let mut sequence = forge_script_sequence::ScriptSequence::default();
        sequence.transactions.push_back(transaction);
        sequence.paths = Some((dir.join("broadcast.json"), dir.join("cache.json")));
        SequenceData::Single(sequence)
    }

    fn load(paths: &(PathBuf, PathBuf), batch: bool) -> Result<RecoveryStore<Ethereum>> {
        let lock = RecoveryLock::acquire(paths)?;
        RecoveryStore::load(paths, batch, lock)?.context("missing recovery snapshot")
    }

    #[test]
    fn snapshot_is_versioned_private_and_locks_writers() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        let store = RecoveryStore::create(data, false).unwrap();
        let value: serde_json::Value = foundry_common::fs::read_json_file(&store.path).unwrap();
        assert_eq!(value["version"], RECOVERY_VERSION);
        assert_eq!(value["deployments"][0]["operations"][0]["id"]["sequence"], 0);
        assert_eq!(value["deployments"][0]["operations"][0]["id"]["index"], 0);
        assert!(value["deployments"][0]["operations"][0]["fingerprint"].is_string());
        assert!(value["deployments"][0]["operations"][0].get("transaction").is_none());
        assert!(RecoveryLock::acquire(&paths).is_err());
        #[cfg(unix)]
        assert_eq!(store.path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn snapshot_recovers_without_compatibility_exports() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        let hash = B256::repeat_byte(0x11);
        {
            let mut store = RecoveryStore::create(data, false).unwrap();
            let deployment = &mut store.data_mut().sequences_mut()[0];
            deployment.transactions[0].hash = Some(hash);
            deployment.pending.push(hash);
            store.save().unwrap();
        }
        for path in [&paths.0, &paths.1] {
            assert!(!path.exists());
        }

        let store = load(&paths, false).unwrap();
        let deployment = &store.data().sequences()[0];
        assert_eq!(deployment.transactions[0].hash, Some(hash));
        assert_eq!(deployment.pending, [hash]);
        store.data().publish(&paths).unwrap();
        assert!(paths.0.exists());
        assert!(paths.1.exists());
    }

    #[test]
    fn snapshot_ignores_stale_compatibility_exports() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        let hash = B256::repeat_byte(0x22);
        {
            let mut store = RecoveryStore::create(data.clone(), false).unwrap();
            store.data_mut().sequences_mut()[0].transactions[0].hash = Some(hash);
            store.save().unwrap();
        }
        data.publish(&paths).unwrap();

        let store = load(&paths, false).unwrap();
        assert_eq!(store.data().sequences()[0].transactions[0].hash, Some(hash));
    }

    #[test]
    fn corrupt_snapshot_fails_closed_even_with_valid_exports() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        data.publish(&paths).unwrap();
        let snapshot = {
            let store = RecoveryStore::create(data, false).unwrap();
            store.path
        };
        std::fs::write(snapshot, b"{").unwrap();

        assert!(load(&paths, false).err().unwrap().to_string().contains("is corrupt"));
    }

    #[test]
    fn interrupted_snapshot_replacement_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        let hash = B256::repeat_byte(0x33);
        let snapshot = {
            let mut store = RecoveryStore::create(data, false).unwrap();
            store.data_mut().sequences_mut()[0].pending.push(hash);
            write_plan(&pending_path(&store.path), &store.plan).unwrap();
            store.path.clone()
        };

        let store = load(&paths, false).unwrap();
        assert_eq!(store.data().sequences()[0].pending, [hash]);
        assert!(snapshot.exists());
        assert!(!pending_path(&snapshot).exists());
    }

    #[test]
    fn changed_operations_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RecoveryStore::create(sequence(dir.path()), false).unwrap();
        store.data_mut().sequences_mut()[0].transactions.clear();
        assert!(store.save().unwrap_err().to_string().contains("does not match"));
    }

    #[test]
    fn relocation_preserves_the_authoritative_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let source_paths = data.paths();
        let mut store = RecoveryStore::create(data, false).unwrap();
        let destination = dir.path().join("broadcast-cache.json");
        let destination_snapshot = recovery_path_from_sensitive(&destination).unwrap();
        let paths = (dir.path().join("broadcasted.json"), destination);
        let relocation = store.prepare_relocation(&paths).unwrap();
        assert!(RecoveryLock::acquire(&source_paths).is_err());
        store.commit_relocation(relocation).unwrap();
        assert_eq!(store.path, destination_snapshot);
        assert!(destination_snapshot.exists());
        RecoveryLock::acquire(&source_paths).unwrap();
    }
}
