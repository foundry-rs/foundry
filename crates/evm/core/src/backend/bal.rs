//! Block access list acceleration and pre-block system state for transaction forks.

use super::{Backend, Fork, ForkPosition, JournaledState, ReplayInputs, apply_state_changeset};
use crate::{
    bal,
    evm::{BlockEnvFor, FoundryEvmNetwork, SpecFor},
};
use alloy_chains::Chain;
use alloy_consensus::{BlockHeader, Typed2718};
use alloy_eips::{BlockId, eip2935::HISTORY_STORAGE_ADDRESS, eip4788::BEACON_ROOTS_ADDRESS};
use alloy_hardforks::EthereumHardfork;
use alloy_network::{
    AnyNetwork, AnyRpcBlock, AnyRpcTransaction, AnyTxEnvelope, BlockResponse, TransactionResponse,
};
use alloy_primitives::{Address, Bytes, ChainId, map::AddressSet};
use alloy_rpc_types::BlockTransactions;
use eyre::ensure;
use foundry_common::{
    SYSTEM_TRANSACTION_TYPE, is_known_system_sender,
    provider::block_access_list::fetch_block_access_list,
};
use foundry_config::{ExecutionSpec, FoundryHardfork};
use revm::primitives::hardfork::SpecId;

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Applies a canonical BAL only when replay would use the same source and execution rules.
    pub(super) fn try_apply_bal_prestate(
        fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
        replay: &ReplayInputs<FEN>,
        block: &AnyRpcBlock,
        target: &AnyRpcTransaction,
        journaled_state: &mut JournaledState,
        persistent_accounts: &AddressSet,
    ) -> eyre::Result<bool> {
        let spec = Into::<SpecId>::into(replay.evm_env.cfg_env.spec);
        if !replay.networks.execution_network().is_ethereum()
            || replay.networks.is_celo()
            || SpecFor::<FEN>::from_foundry_hardfork(EthereumHardfork::Cancun.into()).is_none()
            || Chain::from_id(fork.source_chain_id).is_arbitrum()
            || !spec.is_enabled_in(SpecId::CANCUN)
        {
            return Ok(false);
        }
        let ForkPosition::AfterBlock { block: parent } = fork.position else {
            return Ok(false);
        };
        let index = bal::validate_target(target, block, parent)?;
        let Some(config) = replay.forks.get_fork_config(replay.fork_id.clone())? else {
            return Ok(false);
        };
        let Some(resolved) = &config.resolved else { return Ok(false) };
        let context = resolved.context();
        if resolved.block() != parent
            || !context.network_profile.execution_network().is_ethereum()
            || context.network_profile.is_celo()
            || context.execution_chain_id != replay.evm_env.cfg_env.chain_id
            || context
                .source_fork_block_number
                .is_some_and(|anchor| block.header().number() <= anchor)
            || !context
                .hardfork
                .or_else(|| {
                    FoundryHardfork::from_chain_and_timestamp(
                        context.source_chain_id,
                        block.header().timestamp(),
                    )
                })
                .is_some_and(|hardfork| {
                    matches!(hardfork, FoundryHardfork::Ethereum(_))
                        && SpecId::from(hardfork) == spec
                })
        {
            return Ok(false);
        }
        let BlockTransactions::Full(transactions) = block.transactions() else {
            unreachable!("validate_target requires full transactions")
        };
        ensure!(
            transactions.iter().all(|tx| {
                matches!(&*tx.inner.inner, AnyTxEnvelope::Ethereum(_))
                    && !is_known_system_sender(tx.from())
                    && tx.ty() != SYSTEM_TRANSACTION_TYPE
            }),
            "BAL prestate requires ordinary Ethereum transactions"
        );

        let provider = config.evm_opts.fork_provider_with_url::<AnyNetwork>(&config.url)?;
        let bal_provider = provider.clone();
        let block_hash = block.header().hash;
        let Some(access_list) = fork.db.db.do_any_request(async move {
            Ok(fetch_block_access_list(&bal_provider, BlockId::hash(block_hash)).await)
        })?
        else {
            return Ok(false);
        };
        // A canonical access list cannot reproduce execution involving locally persistent
        // accounts. Reads are significant too, even if those accounts have no BAL writes.
        ensure!(
            access_list.iter().all(|account| !persistent_accounts.contains(&account.address)),
            "BAL accesses locally persistent accounts"
        );
        bal::validate_transaction_changes(&access_list, &transactions[..=index])?;
        let prepared = bal::prepare_prestate(
            &fork.db,
            access_list,
            index,
            transactions.len(),
            block.header().block_access_list_hash(),
            spec,
        )?;
        let state = fork.db.db.do_any_request(async move {
            prepared.verify_storage_roots(&provider, parent.hash).await
        })?;
        // Journal refresh can read additional parent slots. Publish neither cache nor journals
        // unless those reads succeed, leaving a clean parent available for replay on failure.
        apply_state_changeset(state, journaled_state, fork, persistent_accounts)?;
        Ok(true)
    }

    /// Ordered Ethereum pre-block calls, including when the target is the first transaction.
    pub(super) fn pre_block_system_calls(
        replay: &ReplayInputs<FEN>,
        block: &AnyRpcBlock,
        source_chain_id: ChainId,
    ) -> Vec<(Address, Bytes)> {
        let mut calls = Vec::new();
        if replay.networks.execution_network().is_ethereum()
            && !replay.networks.is_celo()
            && SpecFor::<FEN>::from_foundry_hardfork(EthereumHardfork::Cancun.into()).is_some()
            && !Chain::from_id(source_chain_id).is_arbitrum()
            && block.header().number() > 0
        {
            let spec = Into::<SpecId>::into(replay.evm_env.cfg_env.spec);
            if spec.is_enabled_in(SpecId::PRAGUE) {
                calls.push((HISTORY_STORAGE_ADDRESS, block.header().parent_hash().0.into()));
            }
            if spec.is_enabled_in(SpecId::CANCUN)
                && let Some(root) = block.header().parent_beacon_block_root()
            {
                calls.push((BEACON_ROOTS_ADDRESS, root.0.into()));
            }
        }
        calls
    }
}

#[cfg(test)]
mod tests;
