use crate::sequence::{SequenceData, completed_transaction_prefix};
use alloy_consensus::{Transaction, transaction::SignerRecoverable};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_network::{Network, TransactionBuilder, TransactionResponse};
use alloy_primitives::{B256, Bytes, keccak256};
use eyre::{ContextCompat, Result, WrapErr, bail};
use forge_script_sequence::TransactionWithMetadata;
use foundry_common::{FoundryTransactionBuilder, TransactionMaybeSigned};
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
    deployments: Vec<RecoveryDeployment<N>>,
    data: SequenceData<N>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "N::TransactionRequest: Serialize, N::TxEnvelope: Serialize",
    deserialize = "N::TransactionRequest: for<'de2> Deserialize<'de2>, N::TxEnvelope: for<'de2> Deserialize<'de2>"
))]
struct RecoveryDeployment<N: Network> {
    chain: u64,
    batch_id: Option<u32>,
    operations: Vec<RecoveryOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attempts: Vec<SubmissionAttempt<N::TransactionRequest>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RecoveryOperation {
    id: OperationId,
    fingerprint: B256,
    rpc: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct OperationId {
    sequence: u32,
    index: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SignedPayload {
    pub(crate) payload: Bytes,
    pub(crate) hash: B256,
}

#[derive(Clone, Serialize, Deserialize)]
struct SubmissionAttempt<T> {
    id: B256,
    members: Vec<OperationId>,
    kind: AttemptKind<T>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum AttemptKind<T> {
    Signed { request: Option<T>, payload: SignedPayload },
    Delegated { request: T, status: DelegatedStatus },
    Legacy { hash: B256 },
}

fn batch_members<N: Network>(
    deployment: &RecoveryDeployment<N>,
    first_operation: usize,
) -> Result<Vec<OperationId>> {
    let members = deployment
        .operations
        .get(first_operation..)
        .context("batch operation is not in the recovery snapshot")?
        .iter()
        .map(|operation| operation.id)
        .collect::<Vec<_>>();
    if members.is_empty() {
        bail!("batch submission has no operations");
    }
    Ok(members)
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub(crate) enum DelegatedStatus {
    Prepared,
    Pending { hash: B256 },
    OutcomeUnknown,
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
    ) -> Result<Option<Self>>
    where
        N::TxEnvelope: SignerRecoverable,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
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
        plan.validate_signed_payloads()?;
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
        let mut plan = RecoveryPlan::new(data, batch, generation)?;
        if batch {
            plan.import_legacy_batch_attempts()?;
        }
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

    pub(crate) fn signed_payload(&self, sequence: usize, index: usize) -> Option<&SignedPayload> {
        let deployment = self.plan.deployments.get(sequence)?;
        let id = deployment.operations.get(index)?.id;
        deployment.attempts.iter().find_map(|attempt| {
            if !attempt.members.contains(&id) {
                return None;
            }
            match &attempt.kind {
                AttemptKind::Signed { payload, .. } => Some(payload),
                AttemptKind::Delegated { .. } | AttemptKind::Legacy { .. } => None,
            }
        })
    }

    pub(crate) fn delegated_status(
        &self,
        sequence: usize,
        index: usize,
    ) -> Option<DelegatedStatus> {
        let deployment = self.plan.deployments.get(sequence)?;
        let id = deployment.operations.get(index)?.id;
        deployment.attempts.iter().find_map(|attempt| {
            if !attempt.members.contains(&id) {
                return None;
            }
            match &attempt.kind {
                AttemptKind::Delegated { status, .. } => Some(*status),
                AttemptKind::Signed { .. } | AttemptKind::Legacy { .. } => None,
            }
        })
    }

    pub(crate) fn delegated_attempt_id(&self, sequence: usize, index: usize) -> Option<B256> {
        let deployment = self.plan.deployments.get(sequence)?;
        let operation = deployment.operations.get(index)?.id;
        deployment.attempts.iter().find_map(|attempt| {
            (attempt.members.contains(&operation)
                && matches!(attempt.kind, AttemptKind::Delegated { .. }))
            .then_some(attempt.id)
        })
    }

    pub(crate) fn submission_hashes(&self, sequence: usize) -> Vec<B256> {
        self.plan
            .deployments
            .get(sequence)
            .into_iter()
            .flat_map(|deployment| &deployment.attempts)
            .filter_map(|attempt| match &attempt.kind {
                AttemptKind::Signed { payload, .. } => Some(payload.hash),
                AttemptKind::Delegated { status: DelegatedStatus::Pending { hash }, .. } => {
                    Some(*hash)
                }
                AttemptKind::Legacy { hash } => Some(*hash),
                AttemptKind::Delegated { .. } => None,
            })
            .collect()
    }

    pub(crate) fn signed_hashes(&self, sequence: usize) -> Vec<B256> {
        self.plan
            .deployments
            .get(sequence)
            .into_iter()
            .flat_map(|deployment| &deployment.attempts)
            .filter_map(|attempt| match &attempt.kind {
                AttemptKind::Signed { payload, .. } => Some(payload.hash),
                AttemptKind::Delegated { .. } | AttemptKind::Legacy { .. } => None,
            })
            .collect()
    }

    pub(crate) fn batch_signed_attempt(
        &self,
        sequence: usize,
    ) -> Option<(&N::TransactionRequest, &SignedPayload)> {
        if !self.plan.batch {
            return None;
        }
        self.plan.deployments.get(sequence)?.attempts.iter().find_map(|attempt| {
            let AttemptKind::Signed { request: Some(request), payload } = &attempt.kind else {
                return None;
            };
            Some((request, payload))
        })
    }

    pub(crate) fn batch_first_operation(&self, sequence: usize) -> Option<usize> {
        if !self.plan.batch {
            return None;
        }
        self.plan
            .deployments
            .get(sequence)?
            .attempts
            .first()?
            .members
            .first()
            .map(|member| member.index as usize)
    }

    pub(crate) fn has_batch_submission(&self, sequence: usize) -> bool {
        self.plan.batch
            && self
                .plan
                .deployments
                .get(sequence)
                .is_some_and(|deployment| !deployment.attempts.is_empty())
    }

    pub(crate) fn batch_delegated_status(&self, sequence: usize) -> Option<DelegatedStatus> {
        if !self.plan.batch {
            return None;
        }
        self.plan.deployments.get(sequence)?.attempts.iter().find_map(|attempt| {
            let AttemptKind::Delegated { status, .. } = &attempt.kind else { return None };
            Some(*status)
        })
    }

    pub(crate) fn batch_delegated_request(
        &self,
        sequence: usize,
    ) -> Option<&N::TransactionRequest> {
        if !self.plan.batch {
            return None;
        }
        self.plan.deployments.get(sequence)?.attempts.iter().find_map(|attempt| {
            let AttemptKind::Delegated { request, .. } = &attempt.kind else { return None };
            Some(request)
        })
    }

    pub(crate) fn legacy_batch_hash(&self, sequence: usize) -> Option<B256> {
        if !self.plan.batch {
            return None;
        }
        self.plan.deployments.get(sequence)?.attempts.iter().find_map(|attempt| {
            let AttemptKind::Legacy { hash } = &attempt.kind else { return None };
            Some(*hash)
        })
    }

    pub(crate) fn persist_signed_payload(
        &mut self,
        sequence: usize,
        index: usize,
        payload: Bytes,
    ) -> Result<B256>
    where
        N::TxEnvelope: SignerRecoverable,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let deployment = self
            .plan
            .deployments
            .get(sequence)
            .context("signed payload deployment is not in the recovery snapshot")?;
        let operation = deployment
            .operations
            .get(index)
            .context("signed payload operation is not in the recovery snapshot")?;
        let transaction = &self.plan.data.sequences()[sequence].transactions[index];
        let signed = validate_signed_payload::<N>(payload, transaction, deployment.chain)?;
        if let Some(existing) = self.signed_payload(sequence, index) {
            if existing != &signed {
                bail!("refusing to replace an existing signed payload");
            }
            return Ok(existing.hash);
        }
        let id = operation.id;
        if deployment.attempts.iter().any(|attempt| attempt.members.contains(&id)) {
            bail!("refusing to replace an existing submission attempt");
        }
        self.plan.deployments[sequence].attempts.push(SubmissionAttempt {
            id: B256::random(),
            members: vec![id],
            kind: AttemptKind::Signed { request: None, payload: signed },
        });
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            self.plan.deployments[sequence].attempts.pop();
            return Err(error);
        }
        Ok(self.signed_payload(sequence, index).expect("signed payload was persisted").hash)
    }

    pub(crate) fn persist_delegated_request(
        &mut self,
        sequence: usize,
        index: usize,
        request: N::TransactionRequest,
    ) -> Result<()> {
        let deployment = self
            .plan
            .deployments
            .get(sequence)
            .context("delegated operation is not in the recovery snapshot")?;
        let id = deployment
            .operations
            .get(index)
            .context("delegated operation is not in the recovery snapshot")?
            .id;
        if deployment.attempts.iter().any(|attempt| attempt.members.contains(&id)) {
            bail!("refusing to replace an existing submission attempt");
        }
        self.plan.deployments[sequence].attempts.push(SubmissionAttempt {
            id: B256::random(),
            members: vec![id],
            kind: AttemptKind::Delegated { request, status: DelegatedStatus::Prepared },
        });
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            self.plan.deployments[sequence].attempts.pop();
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn persist_delegated_status(
        &mut self,
        sequence: usize,
        index: usize,
        status: DelegatedStatus,
    ) -> Result<()> {
        let deployment = self
            .plan
            .deployments
            .get_mut(sequence)
            .context("delegated operation is not in the recovery snapshot")?;
        let id = deployment
            .operations
            .get(index)
            .context("delegated operation is not in the recovery snapshot")?
            .id;
        let attempt = deployment
            .attempts
            .iter_mut()
            .find(|attempt| attempt.members.contains(&id))
            .context("delegated operation has no submission attempt")?;
        let AttemptKind::Delegated { status: current, .. } = &mut attempt.kind else {
            bail!("operation has no delegated submission attempt");
        };
        let previous = *current;
        if previous == status {
            return Ok(());
        }
        *current = status;
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            let AttemptKind::Delegated { status, .. } = &mut self.plan.deployments[sequence]
                .attempts
                .iter_mut()
                .find(|attempt| attempt.members.contains(&id))
                .unwrap()
                .kind
            else {
                unreachable!()
            };
            *status = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn clear_delegated_request(&mut self, sequence: usize, index: usize) -> Result<()> {
        let deployment = self
            .plan
            .deployments
            .get_mut(sequence)
            .context("delegated operation is not in the recovery snapshot")?;
        let id = deployment
            .operations
            .get(index)
            .context("delegated operation is not in the recovery snapshot")?
            .id;
        let position = deployment
            .attempts
            .iter()
            .position(|attempt| {
                attempt.members.contains(&id)
                    && matches!(&attempt.kind, AttemptKind::Delegated { .. })
            })
            .context("delegated operation has no submission attempt")?;
        let attempt = deployment.attempts.remove(position);
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            self.plan.deployments[sequence].attempts.insert(position, attempt);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn delegated_attempt_location(&self, id: B256) -> Option<(usize, usize)> {
        self.plan.deployments.iter().enumerate().find_map(|(sequence, deployment)| {
            let attempt = deployment.attempts.iter().find(|attempt| {
                attempt.id == id && matches!(attempt.kind, AttemptKind::Delegated { .. })
            })?;
            let operation = attempt.members.first()?;
            Some((sequence, operation.index as usize))
        })
    }

    pub(crate) fn resolve_delegated_hash(
        &mut self,
        attempt_id: B256,
        hash: B256,
        transaction: &N::TransactionResponse,
    ) -> Result<(usize, usize)>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let (sequence, index) = self
            .delegated_attempt_location(attempt_id)
            .context("no interrupted delegated submission matches --resume-attempt")?;
        let deployment = &self.plan.deployments[sequence];
        let attempt = deployment.attempts.iter().find(|attempt| attempt.id == attempt_id).unwrap();
        let AttemptKind::Delegated { request, status } = &attempt.kind else { unreachable!() };
        if !matches!(status, DelegatedStatus::Prepared | DelegatedStatus::OutcomeUnknown) {
            bail!("delegated submission attempt {attempt_id} does not require resolution");
        }
        validate_delegated_transaction::<N>(transaction, request, deployment.chain, hash)?;
        self.persist_delegated_status(sequence, index, DelegatedStatus::Pending { hash })?;
        Ok((sequence, index))
    }

    pub(crate) fn persist_batch_signed_payload(
        &mut self,
        sequence: usize,
        first_operation: usize,
        request: N::TransactionRequest,
        payload: Bytes,
    ) -> Result<B256>
    where
        N::TxEnvelope: Decodable2718 + Encodable2718,
    {
        let signed = SignedPayload { hash: signed_payload_hash::<N>(&payload)?, payload };
        let members = batch_members(
            self.plan
                .deployments
                .get(sequence)
                .context("batch deployment is not in the recovery snapshot")?,
            first_operation,
        )?;
        if self.has_batch_submission(sequence) {
            bail!("refusing to replace an existing batch submission attempt");
        }
        self.plan.deployments[sequence].attempts.push(SubmissionAttempt {
            id: B256::random(),
            members,
            kind: AttemptKind::Signed { request: Some(request), payload: signed },
        });
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            self.plan.deployments[sequence].attempts.pop();
            return Err(error);
        }
        Ok(self.batch_signed_attempt(sequence).unwrap().1.hash)
    }

    pub(crate) fn persist_batch_delegated_request(
        &mut self,
        sequence: usize,
        first_operation: usize,
        request: N::TransactionRequest,
    ) -> Result<()> {
        let members = batch_members(
            self.plan
                .deployments
                .get(sequence)
                .context("batch deployment is not in the recovery snapshot")?,
            first_operation,
        )?;
        if self.has_batch_submission(sequence) {
            bail!("refusing to replace an existing batch submission attempt");
        }
        self.plan.deployments[sequence].attempts.push(SubmissionAttempt {
            id: B256::random(),
            members,
            kind: AttemptKind::Delegated { request, status: DelegatedStatus::Prepared },
        });
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            self.plan.deployments[sequence].attempts.pop();
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn persist_batch_delegated_status(
        &mut self,
        sequence: usize,
        status: DelegatedStatus,
    ) -> Result<()> {
        let first = self
            .batch_first_operation(sequence)
            .context("batch has no delegated submission attempt")?;
        self.persist_delegated_status(sequence, first, status)
    }

    pub(crate) fn clear_batch_submission(&mut self, sequence: usize) -> Result<()> {
        let attempt = self
            .plan
            .deployments
            .get_mut(sequence)
            .context("batch deployment is not in the recovery snapshot")?
            .attempts
            .pop();
        let Some(attempt) = attempt else { return Ok(()) };
        if let Err(error) = write_snapshot(&self.path, &self.plan) {
            self.plan.deployments[sequence].attempts.push(attempt);
            return Err(error);
        }
        Ok(())
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
    N::TransactionRequest: for<'de> Deserialize<'de> + Serialize,
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
                    attempts: Vec::new(),
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

    fn import_legacy_batch_attempts(&mut self) -> Result<()> {
        for (deployment, data) in self.deployments.iter_mut().zip(self.data.sequences().iter()) {
            if data.pending.is_empty() {
                continue;
            }
            let [hash] = data.pending.as_slice() else {
                bail!("cannot import legacy batch progress with multiple pending hashes");
            };
            let first = data
                .transactions
                .iter()
                .position(|transaction| transaction.hash == Some(*hash))
                .context("legacy batch hash is not bound to a recovery operation")?;
            if completed_transaction_prefix(data)? != first {
                bail!("legacy batch progress omits an incomplete operation");
            }
            if data
                .transactions
                .iter()
                .skip(first)
                .any(|transaction| transaction.hash != Some(*hash))
            {
                bail!("cannot import inconsistent legacy batch operation hashes");
            }
            deployment.attempts.push(SubmissionAttempt {
                id: B256::random(),
                members: deployment.operations[first..]
                    .iter()
                    .map(|operation| operation.id)
                    .collect(),
                kind: AttemptKind::Legacy { hash: *hash },
            });
        }
        Ok(())
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
        let mut saved = self.deployments.clone();
        for deployment in &mut saved {
            deployment.attempts.clear();
        }
        let expected = Self::new(self.data.clone(), self.batch, self.generation)?;
        if serde_json::to_value(saved)? != serde_json::to_value(&expected.deployments)? {
            bail!("recovery snapshot does not match the script operations; refusing to resume");
        }
        Ok(())
    }

    fn validate_signed_payloads(&self) -> Result<()>
    where
        N::TxEnvelope: SignerRecoverable,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        for (sequence, deployment) in self.deployments.iter().enumerate() {
            let data = &self.data.sequences()[sequence];
            if self.batch && deployment.attempts.is_empty() {
                let completed = completed_transaction_prefix(data)?;
                if !data.pending.is_empty()
                    || data
                        .transactions
                        .iter()
                        .skip(completed)
                        .any(|transaction| transaction.hash.is_some())
                {
                    bail!("batch progress exists without a durable submission attempt");
                }
            }
            if self.batch && deployment.attempts.len() > 1 {
                bail!("recovery snapshot contains multiple batch submission attempts");
            }
            let mut claimed = vec![false; deployment.operations.len()];
            for attempt in &deployment.attempts {
                let Some(first) = attempt.members.first() else {
                    bail!("recovery snapshot attempt has invalid operation membership");
                };
                if first.sequence as usize != sequence {
                    bail!("recovery snapshot attempt has invalid operation membership");
                }
                let start = first.index as usize;
                let end = start
                    .checked_add(attempt.members.len())
                    .context("recovery snapshot attempt has invalid operation membership")?;
                let operations = deployment
                    .operations
                    .get(start..end)
                    .context("recovery snapshot attempt has invalid operation membership")?;
                if operations
                    .iter()
                    .map(|operation| operation.id)
                    .ne(attempt.members.iter().copied())
                    || claimed[start..end].iter().any(|claimed| *claimed)
                    || if self.batch {
                        end != deployment.operations.len()
                    } else {
                        attempt.members.len() != 1
                    }
                {
                    bail!("recovery snapshot attempt has invalid operation membership");
                }
                if self.batch {
                    let completed = completed_transaction_prefix(data)?;
                    if completed != start && completed != deployment.operations.len() {
                        bail!("batch submission attempt omits an incomplete operation");
                    }
                    let expected_hash = match &attempt.kind {
                        AttemptKind::Signed { payload, .. } => Some(payload.hash),
                        AttemptKind::Delegated {
                            status: DelegatedStatus::Pending { hash },
                            ..
                        }
                        | AttemptKind::Legacy { hash } => Some(*hash),
                        AttemptKind::Delegated { .. } => None,
                    };
                    if data.pending.len() > 1
                        || data.pending.iter().any(|hash| Some(*hash) != expected_hash)
                        || data
                            .transactions
                            .iter()
                            .skip(start)
                            .filter_map(|transaction| transaction.hash)
                            .any(|hash| Some(hash) != expected_hash)
                    {
                        bail!("batch submission attempt conflicts with script progress");
                    }
                }
                claimed[start..end].fill(true);
                match &attempt.kind {
                    AttemptKind::Signed { request, payload } => {
                        if request.is_some() != self.batch {
                            bail!("recovery snapshot signed attempt has an invalid request");
                        }
                        let validated = if let [operation] = operations
                            && !self.batch
                        {
                            validate_signed_payload::<N>(
                                payload.payload.clone(),
                                &data.transactions[operation.id.index as usize],
                                deployment.chain,
                            )?
                        } else {
                            SignedPayload {
                                hash: signed_payload_hash::<N>(&payload.payload)?,
                                payload: payload.payload.clone(),
                            }
                        };
                        if validated != *payload {
                            bail!("recovery snapshot signed payload does not match its hash");
                        }
                    }
                    AttemptKind::Legacy { .. } if !self.batch => {
                        bail!("non-batch recovery snapshot contains a legacy batch attempt");
                    }
                    AttemptKind::Delegated { .. } | AttemptKind::Legacy { .. } => {}
                }
            }
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

fn signed_payload_hash<N: Network>(payload: &Bytes) -> Result<B256>
where
    N::TxEnvelope: Decodable2718 + Encodable2718,
{
    Ok(N::TxEnvelope::decode_2718_exact(payload)
        .wrap_err("recovery snapshot contains an invalid signed payload")?
        .trie_hash())
}

fn validate_signed_payload<N: Network>(
    payload: Bytes,
    planned: &TransactionWithMetadata<N>,
    chain: u64,
) -> Result<SignedPayload>
where
    N::TxEnvelope: Decodable2718 + Encodable2718 + SignerRecoverable,
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    let envelope = N::TxEnvelope::decode_2718_exact(&payload)
        .wrap_err("recovery snapshot contains an invalid signed payload")?;
    let signer = envelope
        .recover_signer()
        .wrap_err("recovery snapshot signed payload has an invalid signature")?;
    let transaction = planned.tx();
    let chain_matches = match transaction {
        TransactionMaybeSigned::Signed { tx, .. } => {
            envelope.trie_hash() == tx.trie_hash() && tx.chain_id().is_none_or(|id| id == chain)
        }
        TransactionMaybeSigned::Unsigned(_) => envelope.chain_id() == Some(chain),
    };
    if transaction.from() != Some(signer)
        || !chain_matches
        || transaction.nonce() != Some(envelope.nonce())
        || transaction.to() != envelope.to()
        || transaction.value().unwrap_or_default() != envelope.value()
        || transaction.input().cloned().unwrap_or_default() != *envelope.input()
        || transaction.authorization_list().as_deref().unwrap_or_default()
            != envelope.authorization_list().unwrap_or_default()
    {
        bail!("signed payload does not match its planned transaction");
    }
    Ok(SignedPayload { hash: envelope.trie_hash(), payload })
}

fn validate_delegated_transaction<N: Network>(
    transaction: &N::TransactionResponse,
    planned: &N::TransactionRequest,
    chain: u64,
    hash: B256,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if transaction.tx_hash() != hash
        || transaction.from() != planned.from().context("delegated request has no sender")?
        || transaction.chain_id() != Some(chain)
        || planned.chain_id() != Some(chain)
        || transaction.nonce() != planned.nonce().context("delegated request has no nonce")?
        || transaction.to() != planned.to()
        || transaction.value() != planned.value().unwrap_or_default()
        || transaction.input() != planned.input().unwrap_or_default()
        || transaction.authorization_list().unwrap_or_default()
            != planned.authorization_list().map(Vec::as_slice).unwrap_or_default()
    {
        bail!("resolved transaction does not match its delegated submission attempt");
    }
    Ok(())
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
    use alloy_consensus::{TxEnvelope, transaction::Recovered};
    use alloy_network::Ethereum;
    use alloy_primitives::hex;
    use alloy_rpc_types::{Transaction as RpcTransaction, TransactionRequest};

    const SIGNED_TX: &[u8] = &hex!(
        "02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000016480c001a070d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0eca052eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a"
    );
    const OTHER_SIGNED_TX: &[u8] = &hex!(
        "02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c001a0cce9a61187b5d18a89ecd27ec675e3b3f10d37f165627ef89a15a7fe76395ce8a07537f5bffb358ffbef22cda84b1c92f7211723f9e09ae037e81686805d3e5505"
    );

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

    fn signed_sequence(dir: &Path) -> SequenceData<Ethereum> {
        let transactions = [SIGNED_TX, OTHER_SIGNED_TX].map(|payload| {
            let envelope = TxEnvelope::decode_2718_exact(payload).unwrap();
            let from = envelope.recover_signer().unwrap();
            let mut request: TransactionRequest = envelope.into();
            request.from = Some(from);
            TransactionWithMetadata::from_tx_request(TransactionMaybeSigned::new(request))
        });
        let mut sequence = forge_script_sequence::ScriptSequence {
            chain: 1,
            transactions: transactions.into(),
            ..Default::default()
        };
        sequence.paths = Some((dir.join("broadcast.json"), dir.join("cache.json")));
        SequenceData::Single(sequence)
    }

    fn delegated_transaction(payload: &[u8]) -> (TransactionRequest, RpcTransaction) {
        let envelope = TxEnvelope::decode_2718_exact(payload).unwrap();
        let from = envelope.recover_signer().unwrap();
        let mut request: TransactionRequest = envelope.clone().into();
        request.from = Some(from);
        let transaction = RpcTransaction {
            inner: Recovered::new_unchecked(envelope, from),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
            block_timestamp: None,
        };
        (request, transaction)
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

    #[test]
    fn signed_payload_is_immutable_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let data = signed_sequence(dir.path());
        let paths = data.paths();
        let expected_hash = {
            let mut store = RecoveryStore::create(data, false).unwrap();
            let hash = store.persist_signed_payload(0, 0, SIGNED_TX.to_vec().into()).unwrap();
            assert!(store.persist_signed_payload(0, 0, OTHER_SIGNED_TX.to_vec().into()).is_err());
            hash
        };

        let store = load(&paths, false).unwrap();
        let signed = store.signed_payload(0, 0).unwrap();
        assert_eq!(signed.payload.as_ref(), SIGNED_TX);
        assert_eq!(signed.hash, expected_hash);
    }

    #[test]
    fn signed_payload_is_bound_to_its_planned_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RecoveryStore::create(signed_sequence(dir.path()), false).unwrap();

        assert!(store.persist_signed_payload(1, 0, SIGNED_TX.to_vec().into()).is_err());
        assert!(store.persist_signed_payload(0, 1, SIGNED_TX.to_vec().into()).is_err());
    }

    #[test]
    fn delegated_submission_is_immutable_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        let request = <Ethereum as Network>::TransactionRequest::default();
        let hash = B256::repeat_byte(0x42);
        {
            let mut store = RecoveryStore::create(data, false).unwrap();
            store.persist_delegated_request(0, 0, request).unwrap();
            store.persist_delegated_status(0, 0, DelegatedStatus::Pending { hash }).unwrap();
            assert!(store.persist_delegated_request(0, 0, Default::default()).is_err());
        }

        let store = load(&paths, false).unwrap();
        assert!(
            matches!(store.delegated_status(0, 0), Some(DelegatedStatus::Pending { hash: value }) if value == hash)
        );
    }

    #[test]
    fn delegated_resolution_is_bound_to_its_planned_transaction() {
        let (request, transaction) = delegated_transaction(SIGNED_TX);
        let hash = transaction.tx_hash();
        validate_delegated_transaction::<Ethereum>(&transaction, &request, 1, hash).unwrap();

        let (other, _) = delegated_transaction(OTHER_SIGNED_TX);
        assert!(validate_delegated_transaction::<Ethereum>(&transaction, &other, 1, hash).is_err());
    }

    #[test]
    fn definite_non_submission_clears_delegated_request() {
        let dir = tempfile::tempdir().unwrap();
        let data = sequence(dir.path());
        let paths = data.paths();
        let mut store = RecoveryStore::create(data, false).unwrap();
        store.persist_delegated_request(0, 0, Default::default()).unwrap();
        store.clear_delegated_request(0, 0).unwrap();
        assert!(store.delegated_status(0, 0).is_none());
        drop(store);

        assert!(load(&paths, false).unwrap().delegated_status(0, 0).is_none());
    }

    #[test]
    fn batch_attempt_uses_shared_membership_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let data = signed_sequence(dir.path());
        let paths = data.paths();
        let request = TransactionRequest::default();
        {
            let mut store = RecoveryStore::create(data, true).unwrap();
            store
                .persist_batch_signed_payload(0, 0, request.clone(), SIGNED_TX.to_vec().into())
                .unwrap();
            assert!(store.persist_signed_payload(0, 1, OTHER_SIGNED_TX.to_vec().into()).is_err());
        }

        let store = load(&paths, true).unwrap();
        assert_eq!(store.batch_first_operation(0), Some(0));
        assert_eq!(store.batch_signed_attempt(0).unwrap().0, &request);
    }

    #[test]
    fn batch_attempt_rejects_conflicting_progress() {
        let foreign = B256::repeat_byte(0x66);
        for (transaction_hash, pending) in [(Some(foreign), vec![]), (None, vec![foreign])] {
            let dir = tempfile::tempdir().unwrap();
            let data = signed_sequence(dir.path());
            let paths = data.paths();
            {
                let mut store = RecoveryStore::create(data, true).unwrap();
                store
                    .persist_batch_signed_payload(
                        0,
                        0,
                        TransactionRequest::default(),
                        SIGNED_TX.to_vec().into(),
                    )
                    .unwrap();
                let deployment = &mut store.data_mut().sequences_mut()[0];
                deployment.transactions[0].hash = transaction_hash;
                deployment.pending = pending;
                write_snapshot(&store.path, &store.plan).unwrap();
            }

            assert!(load(&paths, true).is_err());
        }
    }

    #[test]
    fn batch_progress_without_an_attempt_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let data = signed_sequence(dir.path());
        let paths = data.paths();
        {
            let mut store = RecoveryStore::create(data, true).unwrap();
            store.data_mut().sequences_mut()[0].pending.push(B256::repeat_byte(0x77));
            write_snapshot(&store.path, &store.plan).unwrap();
        }

        assert!(load(&paths, true).is_err());
    }

    #[test]
    fn legacy_batch_import_requires_one_consistent_pending_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = signed_sequence(dir.path());
        let paths = data.paths();
        let hash = B256::repeat_byte(0x55);
        let deployment = &mut data.sequences_mut()[0];
        for transaction in &mut deployment.transactions {
            transaction.hash = Some(hash);
        }
        deployment.pending.push(hash);
        let store =
            RecoveryStore::import(data, true, RecoveryLock::acquire(&paths).unwrap()).unwrap();
        assert_eq!(store.legacy_batch_hash(0), Some(hash));
        drop(store);

        let other = tempfile::tempdir().unwrap();
        let mut data = signed_sequence(other.path());
        let paths = data.paths();
        let deployment = &mut data.sequences_mut()[0];
        deployment.transactions[1].hash = Some(hash);
        deployment.pending.push(hash);
        assert!(RecoveryStore::import(data, true, RecoveryLock::acquire(&paths).unwrap()).is_err());
    }
}
