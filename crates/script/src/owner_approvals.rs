//! Tempo configurable account senders whose owners approve on separate machines.
//!
//! When the local owner keys of a configurable account sender do not reach its threshold,
//! `--broadcast` first simulates the remaining operations in order up to the account's last
//! transaction and fixes the gas limit of each of the account's transactions. It then sends the
//! operations before the account's first transaction, approves the account's remaining
//! transactions with the local owners and saves them as [`AwaitingOwners`] in the sequence
//! instead of sending them. The other owners sign the printed digests elsewhere. `--resume` adds
//! the approvals given with `--tempo.owner-signature` and, once every saved transaction reaches
//! the threshold, the broadcast sends them in order with `0x05` signatures built from the
//! approvals.

use crate::{
    broadcast::{
        SendTransactionKind, convert_tempo_aa_create, estimate_gas, remaining_operation_indices,
    },
    sequence::ScriptSequenceKind,
};
use alloy_consensus::{BlockHeader, SignableTransaction};
use alloy_eips::{
    BlockNumberOrTag,
    eip2718::{Decodable2718, Encodable2718},
};
use alloy_network::{BlockResponse, Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxHash, map::HashMap};
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
            ConfigurableAccountOwnerKey, combine_approvals, decode_unsigned_tx, has_config,
            parse_owner_signature, prepare_request, reaches_threshold, simulation_spec,
            updates_config,
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

/// The most blocks a node simulates in one `eth_simulateV1` request.
const MAX_SIMULATED_BLOCKS: usize = 256;

/// How to raise a node's limits on one `eth_simulateV1` request.
const GAS_CAP_HINT: &str = "the node may cap the gas of a whole simulation request, such as \
     reth's --rpc.gascap (50,000,000 by default), or its number of simulated blocks; raise them \
     or use an RPC without these limits";

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
        prepare_request::<N>(tx, self.account, simulation_spec(self.account, &self.config, &[]));
    }
}

/// Tells whether a saved transaction's approvals do not reach its threshold yet.
pub(crate) fn awaits_approvals<N: Network>(sequence: &ScriptSequenceKind<N>) -> bool
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    // An entry that does not decode fails when it is sent.
    sequence.sequences().iter().flat_map(|deployment| &deployment.awaiting_owners).any(|entry| {
        decode_entry(entry).is_ok_and(|(_, config, _, approvals)| {
            !reaches_threshold(&config, &approvals.into_keys().collect::<Vec<_>>())
        })
    })
}

/// Returns the payload of operation `index` of `deployment` signed with its owners' approvals,
/// and its hash, when it was saved for its owners. The signature is rebuilt from the saved
/// approvals, as [`combine_approvals`] selects them the same way every time.
pub(crate) fn approved_payload<N: Network>(
    deployment: &ScriptSequence<N>,
    index: usize,
) -> Result<Option<(Bytes, TxHash)>> {
    let Some(entry) = deployment.awaiting_owners.iter().find(|entry| entry.index == index) else {
        return Ok(None);
    };
    let (tx, config, _, approvals) = decode_entry(entry)?;
    let signature = combine_approvals(
        entry.account,
        &config,
        tx.signature_hash(),
        approvals.into_values().collect(),
    )?;
    let payload = Bytes::from(
        TempoTxEnvelope::AA(tx.into_signed(TempoSignature::ConfigurableAccount(signature)))
            .encoded_2718(),
    );
    let hash = N::TxEnvelope::decode_2718_exact(&payload)
        .wrap_err("invalid transaction signed with owner approvals")?
        .trie_hash();
    Ok(Some((payload, hash)))
}

/// A transaction of a configurable account with offline owners whose gas limit is fixed, to be
/// saved for its owners once the operations before it are sent.
pub(crate) struct Awaiting<'a, N: Network> {
    request: N::TransactionRequest,
    index: usize,
    owners: &'a OfflineOwners,
}

/// Fixes the gas limits of the transactions of configurable accounts with offline owners, before
/// anything is sent.
///
/// `transactions` are the remaining operations of sequence `sequence_index`, in order. They are
/// simulated in order up to the last one with offline owners, one per block, so that each gas
/// limit succeeds after the operations before it, which are not sent yet. Fails when one of them
/// fails, when there are more than a node simulates, or when one would change its account's
/// configuration before another transaction of the account.
pub(crate) async fn prepare<'a, N: Network>(
    sequence: &ScriptSequenceKind<N>,
    sequence_index: usize,
    provider: &RootProvider<N>,
    transactions: &[(SendTransactionKind<'a, N>, bool, usize)],
    gas_estimate_multiplier: u64,
) -> Result<Vec<Awaiting<'a, N>>>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
    N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
{
    let offline = |kind: &SendTransactionKind<'a, N>| match kind {
        SendTransactionKind::OfflineOwners(_, owners) => Some(*owners),
        _ => None,
    };
    let Some(last) = transactions.iter().rposition(|(kind, ..)| offline(kind).is_some()) else {
        return Ok(Vec::new());
    };
    let transactions = &transactions[..=last];
    if transactions.len() > MAX_SIMULATED_BLOCKS {
        bail!(
            "{} transactions must be simulated in order to fix the gas limits of the transactions \
             awaiting owner approvals, but a node simulates at most {MAX_SIMULATED_BLOCKS}; split \
             the script so that at most {MAX_SIMULATED_BLOCKS} operations run up to the last \
             transaction of a configurable account with offline owners",
            transactions.len()
        );
    }

    let mut requests = Vec::with_capacity(transactions.len());
    for (position, (kind, is_fixed_gas_limit, index)) in transactions.iter().enumerate() {
        let planned = &sequence.sequences()[sequence_index].transactions[*index];
        let mut request = match kind {
            SendTransactionKind::OfflineOwners(tx, owners) => {
                let mut tx = tx.clone();
                owners.prepare_request::<N>(&mut tx);
                // The owners approve with the account's current configuration, which this
                // transaction would change for the ones after it.
                if updates_config(tx.to(), tx.input())
                    && transactions[position + 1..].iter().any(|(kind, ..)| {
                        offline(kind).is_some_and(|other| other.account == owners.account)
                    })
                {
                    bail!(
                        "transaction {index} changes the configuration of {}, whose owners \
                         approve on other machines, before its other transactions: their \
                         approvals would be void once it is sent, so it must be the account's \
                         last transaction in the script",
                        owners.account
                    );
                }
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
    // The gas limits that the owners approve, which are fixed here.
    let adjusted = transactions
        .iter()
        .map(|(kind, is_fixed_gas_limit, _)| offline(kind).is_some() && !is_fixed_gas_limit)
        .collect::<Vec<_>>();

    // Every simulation runs on the same block, so that the limits fixed stay valid.
    let latest = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("the latest block is missing")?;
    let (block, block_gas_limit) = (latest.header().number(), latest.header().gas_limit());
    for (position, request) in requests.iter().enumerate() {
        if offline(&transactions[position].0).is_some()
            && let Some(valid_before) = request.valid_before()
            && latest.header().timestamp() >= valid_before.get()
        {
            bail!(
                "transaction {} would be saved for owners to approve with the deadline \
                 {valid_before}, which has passed",
                transactions[position].2
            );
        }
    }
    let calls = match simulate(provider, &requests, block).await {
        Ok(calls) => calls,
        // A single transaction does not depend on unsent ones, so it can be estimated without
        // `eth_simulateV1`.
        Err(_) if requests.len() == 1 => {
            if adjusted[0] {
                estimate_gas(&mut requests[0], provider, gas_estimate_multiplier, false, false)
                    .await?;
            }
            return Ok(awaiting(transactions, requests));
        }
        Err(error) => bail!(
            "cannot simulate the transactions in order to fix the gas limits of the ones \
             awaiting owner approvals, so no more transactions are sent: {error}"
        ),
    };
    // The first call that fails, as an error.
    let fails = |calls: &[SimCallResult]| {
        calls.iter().position(|call| !call.status).map(|position| {
            let message = calls[position].error.as_ref().map_or("reverted", |e| e.message.as_str());
            // A node can lower the gas limits to its cap for the whole request.
            let hint = if message.to_lowercase().contains("out of gas") {
                format!("; {GAS_CAP_HINT}")
            } else {
                String::new()
            };
            let error = eyre::eyre!(
                "transaction {} fails when simulated after the transactions before it, so no \
                 more transactions are sent or saved for owners to approve: {message}{hint}",
                transactions[position].2
            );
            (position, error)
        })
    };
    if let Some((_, error)) = fails(&calls) {
        return Err(error);
    }

    // Gas used is not the gas a call needs to start with, as calls keep back 1/64 of their gas,
    // so the limits are checked together, and the first one that fails is raised to the least
    // that succeeds until they all do.
    for (position, call) in calls.iter().enumerate() {
        if adjusted[position] {
            let used = call.max_used_gas.unwrap_or_default().max(call.gas_used);
            let gas_limit = apply_gas_estimate_multiplier(used, Some(gas_estimate_multiplier))?;
            requests[position].set_gas_limit(gas_limit.min(block_gas_limit));
        }
    }
    // Each fixed limit succeeds after the ones before it, so the first failure moves forward.
    let mut fixed = None;
    while let Some((position, error)) = fails(&simulate(provider, &requests, block).await?) {
        let index = transactions[position].2;
        if !adjusted[position] || fixed.is_some_and(|fixed| position <= fixed) {
            return Err(error);
        }
        let gas_limit = least_gas_limit(
            provider,
            &mut requests[..=position],
            block,
            block_gas_limit,
            gas_estimate_multiplier,
        )
        .await
        .wrap_err_with(|| format!("cannot fix the gas limit of transaction {index}"))?;
        requests[position].set_gas_limit(gas_limit);
        fixed = Some(position);
    }
    Ok(awaiting(transactions, requests))
}

/// The transactions with offline owners among `transactions`, with their prepared requests.
fn awaiting<'a, N: Network>(
    transactions: &[(SendTransactionKind<'a, N>, bool, usize)],
    requests: Vec<N::TransactionRequest>,
) -> Vec<Awaiting<'a, N>> {
    transactions
        .iter()
        .zip(requests)
        .filter_map(|((kind, _, index), request)| match kind {
            SendTransactionKind::OfflineOwners(_, owners) => {
                Some(Awaiting { request, index: *index, owners })
            }
            _ => None,
        })
        .collect()
}

/// Saves the transactions of configurable accounts with offline owners that [`prepare`] fixed,
/// approved by their local owners, for the other owners to approve instead of sending them.
/// Other operations are left for `--resume`.
pub(crate) async fn export<N: Network>(
    sequence: &mut ScriptSequenceKind<N>,
    sequence_index: usize,
    provider: &RootProvider<N>,
    awaiting: Vec<Awaiting<'_, N>>,
) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::UnsignedTx: SignableTransaction<Signature>,
{
    let mut next_nonces = HashMap::<Address, u64>::default();
    let mut saved = Vec::with_capacity(awaiting.len());
    for Awaiting { request, index, owners } in awaiting {
        let account = owners.account;
        // The operations before it are sent by now.
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
            index,
            account,
            config: (&owners.config).into(),
            unsigned_tx: unsigned_tx.into(),
            digest,
            approvals,
        });
    }

    sequence.sequences_mut()[sequence_index].awaiting_owners.extend(saved);
    sequence.save(true, false)?;
    report(sequence)
}

/// Simulates `requests` in order on top of `block`, one per block, so that each runs on
/// the state the ones before it leave.
async fn simulate<N: Network>(
    provider: &RootProvider<N>,
    requests: &[N::TransactionRequest],
    block: u64,
) -> Result<Vec<SimCallResult>>
where
    N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
{
    let payload = SimulatePayload {
        block_state_calls: requests
            .iter()
            .map(|request| SimBlock { calls: vec![request.clone()], ..Default::default() })
            .collect(),
        ..Default::default()
    };
    let calls = provider
        .raw_request::<_, Vec<SimulatedBlock<Value>>>(
            "eth_simulateV1".into(),
            (payload, BlockNumberOrTag::Number(block)),
        )
        .await
        .map_err(|error| match error.as_error_resp() {
            Some(response) if response.code == -38026 => eyre::eyre!(
                "eth_simulateV1 failed with -38026 ({}): the transactions use more gas or \
                 blocks than the RPC allows in one request; {GAS_CAP_HINT}",
                response.message
            ),
            _ => error.into(),
        })?
        .into_iter()
        .flat_map(|block| block.calls)
        .collect::<Vec<_>>();
    if calls.len() != requests.len() {
        bail!("eth_simulateV1 returned an unexpected number of results");
    }
    Ok(calls)
}

/// Returns the least gas limit with which the last of `requests` succeeds after the ones before
/// it, found as `eth_estimateGas` does and raised by `multiplier` percent, up to `cap`. The last
/// request fails with its current gas limit.
async fn least_gas_limit<N: Network>(
    provider: &RootProvider<N>,
    requests: &mut [N::TransactionRequest],
    block: u64,
    cap: u64,
    multiplier: u64,
) -> Result<u64>
where
    N::TransactionRequest: FoundryTransactionBuilder<N> + serde::Serialize,
{
    let succeeds = async |requests: &mut [N::TransactionRequest], gas: u64| -> Result<bool> {
        requests.last_mut().expect("a request").set_gas_limit(gas);
        Ok(simulate(provider, requests, block).await?.last().expect("a result").status)
    };
    let mut low = requests.last().and_then(|request| request.gas_limit()).unwrap_or_default();
    if low >= cap || !succeeds(requests, cap).await? {
        bail!(
            "it fails after the transactions before it even with the block gas limit {cap}; \
             {GAS_CAP_HINT}"
        );
    }
    let mut high = cap;
    // Within 1% of the least gas limit, as `eth_estimateGas` does.
    while high - low > (low / 100).max(1) {
        let middle = low + (high - low) / 2;
        if succeeds(requests, middle).await? {
            high = middle;
        } else {
            low = middle;
        }
    }
    Ok(apply_gas_estimate_multiplier(high, Some(multiplier))?.max(high).min(cap))
}

/// Adds the owner approvals in `signatures` to the transactions awaiting their owners, and checks
/// that every one of them that was not sent yet reaches its threshold and can still be sent. The
/// broadcast then sends them in order, signed with the approvals, recording each as a submission
/// attempt only then.
///
/// The valid approvals are saved first, so that they can be collected over several runs, and the
/// same signatures can be given again. Fails before anything is sent when a signature approves
/// none of the transactions, when one of them lacks approvals, or when the account spent a planned
/// nonce. When the account's configuration changed, a deadline passed, or the base fee rose above
/// the fee the owners approved, the unsent transactions of that sequence are dropped, so that the
/// broadcast saves them again for the owners to approve with new digests.
pub(crate) async fn assemble<N: Network>(
    sequence: &mut ScriptSequenceKind<N>,
    signatures: &[String],
    config: &Config,
) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    if sequence.sequences().iter().all(|deployment| deployment.awaiting_owners.is_empty()) {
        if !signatures.is_empty() {
            bail!("--tempo.owner-signature is given, but no transaction awaits owner approvals");
        }
        return Ok(());
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
    for deployment in sequence.sequences_mut() {
        for entry in &mut deployment.awaiting_owners {
            let (_, owners_config, digest, mut approvals) = decode_entry(entry)?;
            for (signature, used) in signatures.iter().zip(&mut used) {
                if let Ok(owner) = signature.recover_signer(&digest)
                    && owners_config.owner_weight(owner).is_some()
                {
                    approvals.insert(owner, signature.clone());
                    *used = true;
                }
            }
            entry.approvals = approvals.values().map(PrimitiveSignature::to_bytes).collect();
        }
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

    // Every transaction left to send is checked, as its account may have sent others since. One
    // with a recorded submission attempt is recovered like any other sent transaction.
    for sequence_index in 0..sequence.sequences().len() {
        let remaining = remaining_operation_indices(sequence, sequence_index);
        let unsent = sequence.sequences()[sequence_index]
            .awaiting_owners
            .iter()
            .map(|entry| entry.index)
            .filter(|&index| {
                remaining.contains(&index)
                    && sequence.signed_payload(sequence_index, index).is_none()
            })
            .collect::<BTreeSet<_>>();
        if unsent.is_empty() {
            continue;
        }
        let provider = ProviderBuilder::<TempoNetwork>::from_config_with_url(
            config,
            sequence.rpc_url(sequence_index),
        )?
        .build()?;
        for entry in &sequence.sequences()[sequence_index].awaiting_owners {
            if !unsent.contains(&entry.index) {
                continue;
            }
            if let Some(reason) = void_reason(&provider, entry).await? {
                sh_warn!(
                    "{reason} after transaction {} was saved for the owners of {}, so their \
                     approvals are void; the unsent transactions are saved again for the owners \
                     to approve, with new digests, when --resume is given the local owner keys \
                     and Tempo options of the original --broadcast",
                    entry.index,
                    entry.account
                )?;
                sequence.sequences_mut()[sequence_index]
                    .awaiting_owners
                    .retain(|entry| !unsent.contains(&entry.index));
                sequence.save(true, false)?;
                break;
            }
        }
    }

    if awaits_approvals(sequence) {
        bail!(
            "transactions await more owner approvals, so nothing was sent\n{}",
            describe(sequence)?
        );
    }
    Ok(())
}

/// Tells why a saved transaction can no longer be sent with its owners' approvals: its
/// account's configuration or nonce, its deadline or the base fee changed on chain.
///
/// The account's planned nonces cannot change, so a spent nonce fails here.
async fn void_reason(
    provider: &RootProvider<TempoNetwork>,
    entry: &AwaitingOwners,
) -> Result<Option<String>> {
    let (tx, owners_config, ..) = decode_entry(entry)?;
    let account = entry.account;
    if !has_config(provider, account, &owners_config).await? {
        return Ok(Some(format!("the configuration of {account} changed")));
    }
    if provider.get_transaction_count(account).await? > tx.nonce {
        bail!(
            "{account} sent nonce {} after transaction {} was saved for its owners, so their \
             approvals are void, and the remaining transactions cannot be sent with the nonces \
             the script planned; broadcast the script anew with --start-fresh",
            tx.nonce,
            entry.index
        );
    }
    let latest = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("the latest block is missing")?;
    if let Some(valid_before) = tx.valid_before
        && latest.header().timestamp() >= valid_before.get()
    {
        return Ok(Some(format!("its deadline {valid_before} passed")));
    }
    if let Some(base_fee) = latest.header().base_fee_per_gas()
        && u128::from(base_fee) > tx.max_fee_per_gas
    {
        return Ok(Some(format!(
            "the base fee rose to {base_fee}, above its max fee per gas {}",
            tx.max_fee_per_gas
        )));
    }
    Ok(None)
}

/// Decodes a saved transaction, its configuration and the owners' approvals, and checks its
/// digest.
fn decode_entry(
    entry: &AwaitingOwners,
) -> Result<(
    TempoTransaction,
    ConfigurableAccountConfig,
    B256,
    BTreeMap<Address, PrimitiveSignature>,
)> {
    let tx = decode_unsigned_tx(&entry.unsigned_tx)?;
    let config = ConfigurableAccountConfig::try_from(entry.config.clone())?;
    let digest = configurable_account_digest(tx.signature_hash(), entry.account, config.version);
    if digest != entry.digest {
        bail!("the saved digest of transaction {} does not match its transaction", entry.index);
    }
    let mut approvals = BTreeMap::new();
    for approval in &entry.approvals {
        let approval = PrimitiveSignature::from_bytes(approval)
            .map_err(|error| eyre::eyre!("invalid saved approval: {error}"))?;
        let owner = approval.recover_signer(&digest).map_err(|_| {
            eyre::eyre!("a saved approval of transaction {} is invalid", entry.index)
        })?;
        approvals.insert(owner, approval);
    }
    Ok((tx, config, digest, approvals))
}

/// Prints the transactions awaiting owner approvals and how to approve and send them.
fn report<N: Network>(sequence: &ScriptSequenceKind<N>) -> Result<()>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    if shell::is_json() {
        let transactions = sequence
            .sequences()
            .iter()
            .flat_map(|deployment| {
                deployment.awaiting_owners.iter().map(
                    |entry| serde_json::json!({ "chain": deployment.chain, "transaction": entry }),
                )
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
    for (sequence_index, deployment) in sequence.sequences().iter().enumerate() {
        let remaining = remaining_operation_indices(sequence, sequence_index);
        for entry in
            deployment.awaiting_owners.iter().filter(|entry| remaining.contains(&entry.index))
        {
            let (tx, config, digest, approvals) = decode_entry(entry)?;
            let approved = approvals.into_keys().collect::<Vec<_>>();
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
                config
                    .owners
                    .iter()
                    .map(|owner| owner.owner)
                    .filter(|owner| !approved.contains(owner)),
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
