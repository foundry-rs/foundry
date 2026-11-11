//! Tempo configurable account senders whose owners approve on separate machines.
//!
//! When the local owner keys of a configurable account sender do not reach its threshold,
//! `--broadcast` sends the operations before the account's first transaction, then fixes the
//! nonce, fees and gas limit of each remaining transaction of the account, approves it with the
//! local owners and saves it as [`AwaitingOwners`] in the sequence instead of sending it. The
//! other owners sign the printed digests elsewhere. `--resume` adds the approvals given with
//! `--tempo.owner-signature` and, once every saved transaction reaches the threshold, assembles
//! their `0x05` signatures into signed payloads, which the broadcast sends in order.

use crate::{
    broadcast::{
        SendTransactionKind, convert_tempo_aa_create, estimate_gas, remaining_operation_indices,
    },
    sequence::ScriptSequenceKind,
};
use alloy_consensus::{BlockHeader, SignableTransaction};
use alloy_eips::{
    BlockId, BlockNumberOrTag,
    eip2718::{Decodable2718, Encodable2718},
};
use alloy_network::{BlockResponse, Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxHash, U256, map::HashMap};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::simulate::{SimBlock, SimCallResult, SimulatePayload, SimulatedBlock};
use alloy_signer::Signature;
use eyre::{Context, ContextCompat, Result, bail};
use forge_script_sequence::{AwaitingOwners, ScriptSequence};
use foundry_cli::utils::apply_gas_estimate_multiplier;
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    shell,
    tempo::{
        configurable_account::{
            ConfigurableAccountOwnerKey, combine_approvals, config_json, decode_unsigned_tx,
            detect_configurable_account, parse_config, parse_owner_signature, reaches_threshold,
            simulation_spec,
        },
        resolve_and_set_fee_token,
    },
};
use foundry_config::Config;
use itertools::Itertools;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tempo_alloy::TempoNetwork;
use tempo_primitives::{
    TempoTxEnvelope,
    transaction::{
        ConfigurableAccountConfig, PrimitiveSignature, TempoSignature, TempoTransaction,
        configurable_account_digest,
    },
};

/// A configurable account sender whose local owner keys do not form a quorum that Tempo accepts,
/// so that the other owners approve its transactions elsewhere.
#[derive(Clone, Debug)]
pub struct OfflineOwners {
    account: Address,
    config: ConfigurableAccountConfig,
    /// The local owner keys, which approve when the transactions are saved.
    keys: Vec<ConfigurableAccountOwnerKey>,
}

impl OfflineOwners {
    /// Returns the owners of `account`, or `None` when the local `keys` form a quorum themselves:
    /// they reach its threshold with at most the number of signatures Tempo accepts.
    pub fn new(
        account: Address,
        config: ConfigurableAccountConfig,
        keys: Vec<ConfigurableAccountOwnerKey>,
    ) -> Option<Self> {
        let owners = keys.iter().map(ConfigurableAccountOwnerKey::address).collect::<Vec<_>>();
        (!reaches_threshold(&config, &owners)).then_some(Self { account, config, keys })
    }

    /// Makes `tx` a Tempo transaction from the account. The owners that will approve are not
    /// known yet, so its gas models the largest quorum.
    fn prepare_request<N: Network>(&self, tx: &mut N::TransactionRequest)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        tx.set_from(self.account);
        if tx.nonce_key().is_none() {
            tx.set_nonce_key(U256::ZERO);
        }
        tx.set_configurable_account_simulation(simulation_spec(self.account, &self.config, &[]));
    }
}

/// The saved transactions that still await their owners' approvals, by sequence and position in
/// `awaiting_owners`.
pub(crate) fn pending<N: Network>(
    sequence: &ScriptSequenceKind<N>,
) -> impl Iterator<Item = (usize, usize)>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    sequence.sequences().iter().enumerate().flat_map(|(sequence_index, deployment)| {
        deployment
            .awaiting_owners
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.signed.is_none())
            .map(move |(position, _)| (sequence_index, position))
    })
}

/// Returns the payload of operation `index` of `deployment` signed with its owners' approvals,
/// and its hash, once `--resume` has assembled it.
pub(crate) fn approved_payload<N: Network>(
    deployment: &ScriptSequence<N>,
    index: usize,
) -> Result<Option<(Bytes, TxHash)>> {
    let Some(payload) = deployment
        .awaiting_owners
        .iter()
        .find(|entry| entry.index == index)
        .and_then(|entry| entry.signed.clone())
    else {
        return Ok(None);
    };
    let hash = N::TxEnvelope::decode_2718_exact(&payload)
        .wrap_err("invalid transaction signed with owner approvals")?
        .trie_hash();
    Ok(Some((payload, hash)))
}

/// Saves the transactions of configurable accounts with offline owners for their owners to
/// approve, instead of sending them.
///
/// `transactions` are the remaining operations of sequence `sequence_index`, in order, from the
/// first one with offline owners. They are simulated in order on top of the chain, so that each
/// gas limit is the least that succeeds after the operations before it, which are not sent yet.
/// Other operations are left for `--resume`.
pub(crate) async fn export<N: Network>(
    sequence: &mut ScriptSequenceKind<N>,
    sequence_index: usize,
    provider: &RootProvider<N>,
    transactions: &[(SendTransactionKind<'_, N>, bool, usize)],
    gas_estimate_multiplier: u64,
) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
    N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
    N::UnsignedTx: SignableTransaction<Signature>,
{
    let mut requests = Vec::with_capacity(transactions.len());
    for (kind, is_fixed_gas_limit, index) in transactions {
        let planned = &sequence.sequences()[sequence_index].transactions[*index];
        let mut request = match kind {
            SendTransactionKind::OfflineOwners(tx, owners) => {
                let mut tx = tx.clone();
                owners.prepare_request::<N>(&mut tx);
                tx
            }
            SendTransactionKind::ConfigurableAccount(tx, account) => {
                let mut tx = tx.clone();
                account.prepare_request::<N>(&mut tx);
                tx
            }
            SendTransactionKind::Raw(tx, _)
            | SendTransactionKind::Unlocked(tx)
            | SendTransactionKind::Browser(tx, _)
            | SendTransactionKind::AccessKey(tx, _) => tx.clone(),
            SendTransactionKind::Signed(tx) => tx.clone().into(),
            SendTransactionKind::PreparedRaw(payload, _) => {
                N::TxEnvelope::decode_2718_exact(payload)?.into()
            }
        };
        if let Some(from) = planned.tx().from() {
            request.set_from(from);
        }
        let from = request.from();
        resolve_and_set_fee_token(Some(provider), &mut request, from).await?;
        convert_tempo_aa_create::<N>(&mut request);
        if !is_fixed_gas_limit {
            request.reset_gas_limit();
        }
        requests.push(request);
    }

    let block_gas_limit = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("the latest block is missing")?
        .header()
        .gas_limit();
    // The node runs each operation on the state the ones before it leave, with the placeholder
    // quorum of each configurable account.
    let simulated = match simulate(provider, &requests, block_gas_limit).await {
        Ok(calls) => Some(calls),
        // Without it, a transaction that depends on an earlier unsent one cannot be estimated, and
        // its owners would approve a gas limit that runs out.
        Err(error)
            if transactions[1..]
                .iter()
                .any(|(kind, ..)| matches!(kind, SendTransactionKind::OfflineOwners(..))) =>
        {
            bail!(
                "the RPC cannot simulate the transactions awaiting owner approvals in order \
                 ({error}), so their gas limits cannot be fixed before the owners approve them"
            )
        }
        Err(_) => None,
    };

    let mut next_nonces = HashMap::<Address, u64>::default();
    let mut saved = Vec::new();
    for (position, (kind, is_fixed_gas_limit, index)) in transactions.iter().enumerate() {
        let mut request = requests[position].clone();
        if let Some(call) = simulated.as_ref().map(|calls| &calls[position])
            && !call.status
        {
            bail!(
                "transaction {index} fails when simulated after the transactions before it, so \
                 nothing is saved for owners to approve: {}",
                call.error.as_ref().map_or("reverted", |error| error.message.as_str())
            );
        }
        let SendTransactionKind::OfflineOwners(_, owners) = kind else { continue };
        let account = owners.account;

        let nonce = request.nonce().context("transaction is missing its planned nonce")?;
        let expected = match next_nonces.get(&account) {
            Some(nonce) => *nonce,
            None => provider.get_transaction_count(account).await?,
        };
        if nonce != expected {
            bail!(
                "transaction {index} of {account} was planned with nonce {nonce}, but the \
                 account's next nonce is {expected}"
            );
        }
        next_nonces.insert(account, nonce + 1);

        // The owners approve the gas limit, so it is fixed now. The in-order simulation accounts
        // for the operations before it, which are not sent yet; an estimate on the current state
        // only does for the first operation, so it is only used then, or without a simulation.
        // Gas used is not the gas a call needs to start with, as calls keep back 1/64 of their
        // gas, so the limit is then checked by simulating it after the operations before it.
        if !is_fixed_gas_limit {
            let simulated = simulated
                .as_ref()
                .map(|calls| {
                    let call = &calls[position];
                    apply_gas_estimate_multiplier(
                        call.max_used_gas.unwrap_or_default().max(call.gas_used),
                        Some(gas_estimate_multiplier),
                    )
                })
                .transpose()?;
            let estimated = if position == 0 || simulated.is_none() {
                let mut estimated = request.clone();
                estimate_gas(&mut estimated, provider, gas_estimate_multiplier, false, false)
                    .await?;
                estimated.gas_limit()
            } else {
                None
            };
            let mut gas_limit = simulated.max(estimated).expect("estimated or simulated");
            if simulated.is_some() {
                gas_limit = least_gas_limit(
                    provider,
                    &requests[..=position],
                    gas_limit,
                    block_gas_limit,
                    gas_estimate_multiplier,
                )
                .await
                .wrap_err_with(|| format!("cannot fix the gas limit of transaction {index}"))?;
            }
            request.set_gas_limit(gas_limit);
            requests[position].set_gas_limit(gas_limit);
        }

        let unsigned = request.build_unsigned().map_err(|error| eyre::eyre!("{}", error.error))?;
        let unsigned_tx = unsigned.encoded_for_signing();
        let digest =
            configurable_account_digest(unsigned.signature_hash(), account, owners.config.version);
        // A `0x05` signature only signs Tempo (`0x76`) transactions.
        decode_unsigned_tx(&unsigned_tx)?;
        let mut approvals = Vec::with_capacity(owners.keys.len());
        for key in &owners.keys {
            let approval = key.sign(&digest).await.wrap_err_with(|| {
                format!("owner {} failed to approve transaction {index}", key.address())
            })?;
            approvals.push(approval.to_bytes());
        }
        saved.push(AwaitingOwners {
            index: *index,
            account,
            config: config_json(&owners.config),
            unsigned_tx: unsigned_tx.into(),
            digest,
            approvals,
            signed: None,
        });
    }

    sequence.sequences_mut()[sequence_index].awaiting_owners.extend(saved);
    sequence.save(true, false)?;
    report(sequence)
}

/// Simulates `requests` in order on top of the latest block, so that each runs on the state the
/// ones before it leave.
///
/// A node simulates at most 256 blocks, so the calls share blocks of `block_gas_limit` gas, in
/// order, while the gas they are given fits in the block. A call with a gas limit is given it, and
/// one without is given the largest share of the block that keeps the blocks within the bound, or
/// the whole block when it is alone in it. A call that fails with its share, or that the node
/// rejects with it, is simulated again alone in a block.
async fn simulate<N: Network>(
    provider: &RootProvider<N>,
    requests: &[N::TransactionRequest],
    block_gas_limit: u64,
) -> Result<Vec<SimCallResult>>
where
    N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
{
    const MAX_BLOCKS: usize = 256;
    let mut alone = BTreeSet::new();
    loop {
        let layout = |share: u64| {
            let mut blocks = Vec::<Vec<usize>>::new();
            let mut reserved = block_gas_limit;
            for (position, request) in requests.iter().enumerate() {
                let gas = if alone.contains(&position) {
                    block_gas_limit
                } else {
                    request.gas_limit().unwrap_or(share)
                };
                if reserved.saturating_add(gas) > block_gas_limit {
                    blocks.push(Vec::new());
                    reserved = 0;
                }
                reserved = reserved.saturating_add(gas);
                blocks.last_mut().expect("a block").push(position);
            }
            blocks
        };
        // The blocks only grow in number with the share, so the largest share that fits is
        // searched for.
        if layout(1).len() > MAX_BLOCKS {
            bail!(
                "the transactions need more than {MAX_BLOCKS} blocks of gas limit \
                 {block_gas_limit} to simulate in order"
            );
        }
        let (mut low, mut high) = (1, block_gas_limit);
        while low < high {
            let middle = high - (high - low) / 2;
            if layout(middle).len() <= MAX_BLOCKS {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let share = low;
        let blocks = layout(share);

        let shared = |block: &[usize], position: usize| {
            block.len() > 1 && requests[position].gas_limit().is_none()
        };
        // Simulates the first `count` calls.
        let run = async |count: usize| {
            let payload = SimulatePayload {
                block_state_calls: blocks
                    .iter()
                    .filter(|block| block[0] < count)
                    .map(|block| SimBlock {
                        calls: block
                            .iter()
                            .filter(|&&position| position < count)
                            .map(|&position| {
                                let mut request = requests[position].clone();
                                if shared(block, position) {
                                    request.set_gas_limit(share);
                                }
                                request
                            })
                            .collect(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            provider
                .raw_request::<_, Vec<SimulatedBlock<Value>>>(
                    "eth_simulateV1".into(),
                    (payload, "latest"),
                )
                .await
        };
        let simulated = match run(requests.len()).await {
            Ok(simulated) => simulated,
            // The node rejects every call when one is given less gas than its intrinsic gas, so
            // the first call it rejects is found by simulating shorter prefixes.
            Err(error) => {
                let (mut low, mut high) = (0, requests.len().saturating_sub(1));
                while low < high {
                    let middle = low + (high - low) / 2;
                    if run(middle + 1).await.is_err() {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                if !blocks.iter().any(|block| block.contains(&low) && shared(block, low)) {
                    return Err(error.into());
                }
                alone.insert(low);
                continue;
            }
        };
        if simulated.len() != blocks.len()
            || simulated
                .iter()
                .zip(&blocks)
                .any(|(result, block)| result.calls.len() != block.len())
        {
            bail!("eth_simulateV1 returned an unexpected number of results");
        }
        let calls = simulated.into_iter().flat_map(|block| block.calls).collect::<Vec<_>>();

        let retried = blocks
            .iter()
            .flat_map(|block| block.iter().filter(|&&position| shared(block, position)))
            .filter(|&&position| !calls[position].status)
            .copied()
            .collect::<Vec<_>>();
        if retried.is_empty() {
            return Ok(calls);
        }
        alone.extend(retried);
    }
}

/// Returns `gas_limit` when the last of `requests` succeeds with it after the ones before it, and
/// otherwise the least gas limit with which it succeeds, found as `eth_estimateGas` does and raised
/// by `multiplier` percent, up to `cap`.
async fn least_gas_limit<N: Network>(
    provider: &RootProvider<N>,
    requests: &[N::TransactionRequest],
    gas_limit: u64,
    cap: u64,
    multiplier: u64,
) -> Result<u64>
where
    N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
{
    let mut requests = requests.to_vec();
    let succeeds = async |requests: &mut [N::TransactionRequest], gas: u64| -> Result<bool> {
        requests.last_mut().expect("a request").set_gas_limit(gas);
        Ok(simulate(provider, requests, cap).await?.last().expect("a result").status)
    };
    let gas_limit = gas_limit.min(cap);
    if succeeds(&mut requests, gas_limit).await? {
        return Ok(gas_limit);
    }
    if gas_limit == cap || !succeeds(&mut requests, cap).await? {
        bail!("it fails after the transactions before it even with the block gas limit {cap}");
    }
    let (mut low, mut high) = (gas_limit, cap);
    // Within 1% of the least gas limit, as `eth_estimateGas` does.
    while high - low > (low / 100).max(1) {
        let middle = low + (high - low) / 2;
        if succeeds(&mut requests, middle).await? {
            high = middle;
        } else {
            low = middle;
        }
    }
    Ok(apply_gas_estimate_multiplier(high, Some(multiplier))?.min(cap))
}

/// Adds the owner approvals in `signatures` to the transactions awaiting their owners and, once
/// every one of them reaches its threshold, signs them with the approvals. The broadcast sends
/// them in order, recording each as a submission attempt only then.
///
/// The valid approvals are saved first, so that they can be collected over several runs. Fails
/// before anything is signed or sent when a signature approves none of the transactions, when one
/// of them lacks approvals, or when one can no longer be sent: the account's configuration
/// changed, its nonce moved on, its deadline passed, or the base fee rose above the fee the
/// owners approved. Every run checks again the transactions signed by an earlier run that were not
/// sent yet, as their accounts may have sent other transactions since.
pub(crate) async fn assemble<N: Network>(
    sequence: &mut ScriptSequenceKind<N>,
    signatures: &[String],
    config: &Config,
) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    let pending = pending(sequence).collect::<Vec<_>>();
    if pending.is_empty() {
        if !signatures.is_empty() {
            bail!("--tempo.owner-signature is given, but no transaction awaits owner approvals");
        }
        return check_unsent(sequence, config).await;
    }
    let signatures = signatures
        .iter()
        .enumerate()
        .map(|(position, value)| {
            parse_owner_signature(value)
                .wrap_err_with(|| format!("invalid --tempo.owner-signature {}", position + 1))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut used = vec![false; signatures.len()];
    let mut complete = true;
    let mut collected = Vec::with_capacity(pending.len());
    for &(sequence_index, position) in &pending {
        let entry = &mut sequence.sequences_mut()[sequence_index].awaiting_owners[position];
        let (_, owners_config, digest) = decode_entry(entry)?;
        let mut approvals = BTreeMap::new();
        for approval in &entry.approvals {
            let approval = PrimitiveSignature::from_bytes(approval)
                .map_err(|error| eyre::eyre!("invalid saved approval: {error}"))?;
            let owner = approval.recover_signer(&digest).map_err(|_| {
                eyre::eyre!("a saved approval of transaction {} is invalid", entry.index)
            })?;
            approvals.insert(owner, approval);
        }
        for (signature, used) in signatures.iter().zip(&mut used) {
            if let Ok(owner) = signature.recover_signer(&digest)
                && owners_config.owner_weight(owner).is_some()
            {
                approvals.insert(owner, signature.clone());
                *used = true;
            }
        }
        entry.approvals = approvals.values().map(PrimitiveSignature::to_bytes).collect();
        complete &=
            reaches_threshold(&owners_config, &approvals.keys().copied().collect::<Vec<_>>());
        collected.push(approvals.into_values().collect::<Vec<_>>());
    }
    sequence.save(true, false)?;

    if let Some(position) = used.iter().position(|used| !used) {
        bail!(
            "--tempo.owner-signature {} is not an owner's approval of any transaction awaiting \
             owner approvals: it must sign one of their digests with an owner key\n{}",
            position + 1,
            describe(sequence)?
        );
    }
    if !complete {
        bail!(
            "transactions await more owner approvals, so nothing was sent\n{}",
            describe(sequence)?
        );
    }

    // Every transaction is checked before any is signed, and none is recorded as a submission
    // attempt until the broadcast sends it in order.
    check_unsent(sequence, config).await?;
    let mut signed = Vec::with_capacity(pending.len());
    for (&(sequence_index, position), approvals) in pending.iter().zip(collected) {
        let entry = &sequence.sequences()[sequence_index].awaiting_owners[position];
        let (tx, owners_config, _) = decode_entry(entry)?;
        let signature =
            combine_approvals(entry.account, &owners_config, tx.signature_hash(), approvals)?;
        let payload =
            TempoTxEnvelope::AA(tx.into_signed(TempoSignature::ConfigurableAccount(signature)))
                .encoded_2718();
        signed.push((sequence_index, position, Bytes::from(payload)));
    }
    for (sequence_index, position, payload) in signed {
        sequence.sequences_mut()[sequence_index].awaiting_owners[position].signed = Some(payload);
    }
    sequence.save(true, false)
}

/// Checks that every transaction saved for its owners and not sent yet can still be sent with
/// their approvals, whether or not an earlier run signed it.
async fn check_unsent<N: Network>(sequence: &ScriptSequenceKind<N>, config: &Config) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    for (sequence_index, deployment) in sequence.sequences().iter().enumerate() {
        let remaining = remaining_operation_indices(sequence, sequence_index);
        for entry in &deployment.awaiting_owners {
            // A recorded submission attempt is recovered like any other sent transaction.
            if remaining.contains(&entry.index)
                && sequence.signed_payload(sequence_index, entry.index).is_none()
            {
                check_sendable(sequence, sequence_index, entry, config).await?;
            }
        }
    }
    Ok(())
}

/// Checks that the configuration, nonce, deadline and fees of a saved transaction are still valid
/// on chain.
async fn check_sendable<N: Network>(
    sequence: &ScriptSequenceKind<N>,
    sequence_index: usize,
    entry: &AwaitingOwners,
    config: &Config,
) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    let (tx, owners_config, _) = decode_entry(entry)?;
    let account = entry.account;
    let void = |reason: String| {
        eyre::eyre!(
            "{reason} after transaction {} was saved for the owners of {account}, so their \
             approvals are void; run the script again with --broadcast --start-fresh",
            entry.index
        )
    };
    let provider = ProviderBuilder::<TempoNetwork>::from_config_with_url(
        config,
        sequence.rpc_url(sequence_index),
    )?
    .build()?;
    detect_configurable_account(
        &provider,
        account,
        &[],
        Some(owners_config.clone()),
        true,
        BlockId::latest(),
    )
    .await
    .map_err(|error| void(format!("the configuration of {account} changed ({error})")))?;
    let nonce = provider.get_transaction_count(account).await?;
    if nonce > tx.nonce {
        return Err(void(format!("{account} sent nonce {}", tx.nonce)));
    }
    let latest = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("the latest block is missing")?;
    if let Some(valid_before) = tx.valid_before
        && latest.header().timestamp() >= valid_before.get()
    {
        return Err(void(format!("its deadline {valid_before} passed")));
    }
    if let Some(base_fee) = latest.header().base_fee_per_gas()
        && u128::from(base_fee) > tx.max_fee_per_gas
    {
        return Err(void(format!(
            "the base fee rose to {base_fee}, above its max fee per gas {}",
            tx.max_fee_per_gas
        )));
    }
    Ok(())
}

/// Decodes a saved transaction and its configuration, and checks its digest.
fn decode_entry(
    entry: &AwaitingOwners,
) -> Result<(TempoTransaction, ConfigurableAccountConfig, B256)> {
    let tx = decode_unsigned_tx(&entry.unsigned_tx)?;
    let config = parse_config(&entry.config.to_string())?;
    let digest = configurable_account_digest(tx.signature_hash(), entry.account, config.version);
    if digest != entry.digest {
        bail!("the saved digest of transaction {} does not match its transaction", entry.index);
    }
    Ok((tx, config, digest))
}

/// Prints the transactions awaiting owner approvals and how to approve and send them.
fn report<N: Network>(sequence: &ScriptSequenceKind<N>) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    if shell::is_json() {
        let transactions = pending(sequence)
            .map(|(sequence_index, position)| {
                let deployment = &sequence.sequences()[sequence_index];
                let entry = &deployment.awaiting_owners[position];
                serde_json::json!({ "chain": deployment.chain, "transaction": entry })
            })
            .collect::<Vec<_>>();
        sh_println!(
            "{}",
            serde_json::json!({ "status": "awaiting_owner_approvals", "transactions": transactions })
        )?;
        return Ok(());
    }
    sh_println!("\n{}", describe(sequence)?)?;
    Ok(())
}

/// Describes the transactions awaiting owner approvals: for each, its digest and the owners that
/// approved it, then how the other owners sign and how to send the transactions.
fn describe<N: Network>(sequence: &ScriptSequenceKind<N>) -> Result<String>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    let mut lines = vec!["==========================".to_string()];
    // For each account, its configuration and the owners missing from any of its transactions.
    let mut accounts = BTreeMap::<Address, (ConfigurableAccountConfig, Vec<Address>)>::new();
    for (sequence_index, position) in pending(sequence) {
        let deployment = &sequence.sequences()[sequence_index];
        let entry = &deployment.awaiting_owners[position];
        let (tx, config, digest) = decode_entry(entry)?;
        let approved = entry
            .approvals
            .iter()
            .filter_map(|approval| PrimitiveSignature::from_bytes(approval).ok())
            .filter_map(|approval| approval.recover_signer(&digest).ok())
            .collect::<Vec<_>>();
        if !accounts.contains_key(&entry.account) {
            lines.push(format!(
                "\nTransactions of the Tempo configurable account {} on chain {} await approvals \
                 from its owners, with threshold {}:",
                entry.account, deployment.chain, config.threshold
            ));
        }
        let (_, missing) =
            accounts.entry(entry.account).or_insert_with(|| (config.clone(), Vec::new()));
        missing.extend(
            config.owners.iter().map(|owner| owner.owner).filter(|owner| !approved.contains(owner)),
        );
        let weight = approved
            .iter()
            .unique()
            .filter_map(|owner| config.owner_weight(*owner))
            .map(u16::from)
            .sum::<u16>();
        let approved = if approved.is_empty() {
            "no approvals yet".to_string()
        } else {
            format!("approved by {}", approved.iter().join(", "))
        };
        lines.push(format!(
            "  transaction {} (nonce {}, gas limit {}): digest {digest}, {approved} (weight \
             {weight} of {})",
            entry.index, tx.nonce, tx.gas_limit, config.threshold
        ));
    }
    for (account, (config, missing)) in accounts {
        let missing = missing
            .into_iter()
            .unique()
            .map(|owner| format!("{owner} (weight {})", config.owner_weight(owner).unwrap_or(0)))
            .join(", ");
        lines.push(format!(
            "\nOwners of {account} that have not approved every transaction: {missing}"
        ));
    }
    lines.push(
        "\nEach owner signs every digest without hashing it again, with `cast wallet sign \
         --no-hash <DIGEST>` and their wallet options, or with a passkey: a WebAuthn assertion \
         whose challenge is the digest, saved as JSON with the credential's public key. Then send \
         the transactions by running the same command with `--resume` instead of `--broadcast`, \
         adding `--tempo.owner-signature <SIGNATURE>` for each approval. Approvals are kept once \
         given, so they can be collected over several runs."
            .to_string(),
    );
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_signer_local::PrivateKeySigner;
    use foundry_wallets::WalletSigner;
    use std::sync::Arc;
    use tempo_primitives::transaction::ConfigurableAccountOwner;

    /// Nine local weight-1 owners reach threshold 9 by weight, but Tempo accepts at most eight
    /// signatures, so the offline weight-9 owner approves.
    #[test]
    fn local_owners_beyond_signature_limit_are_offline() {
        let signers = (1..=10u8)
            .map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap())
            .collect::<Vec<_>>();
        let mut owners = signers
            .iter()
            .enumerate()
            .map(|(i, signer)| ConfigurableAccountOwner {
                owner: signer.address(),
                weight: if i == 9 { 9 } else { 1 },
            })
            .collect::<Vec<_>>();
        owners.sort_by_key(|owner| owner.owner);
        let config =
            ConfigurableAccountConfig { salt: B256::ZERO, version: 0, threshold: 9, owners };
        let keys = |count: usize| {
            signers[..count]
                .iter()
                .map(|signer| {
                    ConfigurableAccountOwnerKey::Wallet(Arc::new(WalletSigner::Local(
                        signer.clone(),
                    )))
                })
                .collect::<Vec<_>>()
        };
        let account = Address::repeat_byte(0x42);
        assert!(OfflineOwners::new(account, config.clone(), keys(9)).is_some());
        assert!(OfflineOwners::new(account, config, keys(10)).is_none());
    }
}
