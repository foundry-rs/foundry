use crate::{
    Cheatcode, Cheatcodes, CheatcodesCore, CheatcodesExecutor, CheatsCtxt, Result, Vm::*,
    json::json_value_to_token,
};
use alloy_dyn_abi::DynSolValue;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, B256, U256, map::AddressHashMap};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::Filter;
use alloy_sol_types::SolValue;
use foundry_common::provider::ProviderBuilder;
use foundry_evm_core::{
    backend::{Backend, ForkAccountField, LocalForkId},
    evm::{ExecutionPhase, FoundryEvmFor, FoundryEvmNetwork},
    fork::CreateFork,
};

impl Cheatcode for activeForkCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.active_fork_id().map(|id| id.abi_encode()).ok_or_else(|| fmt_err!("no active fork"))
    }
}

impl Cheatcode for createFork_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { urlOrAlias } = self;
        create_fork(ccx, urlOrAlias, None)
    }
}

impl Cheatcode for createFork_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { urlOrAlias, blockNumber } = self;
        create_fork(ccx, urlOrAlias, Some(blockNumber.saturating_to()))
    }
}

impl Cheatcode for createFork_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { urlOrAlias, txHash } = self;
        create_fork_at_transaction(ccx, urlOrAlias, txHash)
    }
}

impl Cheatcode for createSelectFork_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { urlOrAlias } = self;
        create_select_fork(ccx, urlOrAlias, None)
    }
}

impl Cheatcode for createSelectFork_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { urlOrAlias, blockNumber } = self;
        create_select_fork(ccx, urlOrAlias, Some(blockNumber.saturating_to()))
    }
}

impl Cheatcode for createSelectFork_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { urlOrAlias, txHash } = self;
        create_select_fork_at_transaction(ccx, urlOrAlias, txHash)
    }
}

impl Cheatcode for rollFork_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { blockNumber } = self;
        persist_caller(ccx);
        let result = fork_op(ccx, |db, evm| {
            db.roll_fork(None, (*blockNumber).to(), evm, ExecutionPhase::InFlight)
        })?;
        record_fork_roll(ccx, None);
        Ok(result)
    }
}

impl Cheatcode for rollFork_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { txHash } = self;
        persist_caller(ccx);
        let result = fork_op(ccx, |db, evm| {
            db.roll_fork_to_transaction(None, *txHash, evm, ExecutionPhase::InFlight)
        })?;
        record_fork_roll(ccx, None);
        Ok(result)
    }
}

impl Cheatcode for rollFork_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { forkId, blockNumber } = self;
        persist_caller(ccx);
        let result = fork_op(ccx, |db, evm| {
            db.roll_fork(Some(*forkId), (*blockNumber).to(), evm, ExecutionPhase::InFlight)
        })?;
        record_fork_roll(ccx, Some(*forkId));
        Ok(result)
    }
}

impl Cheatcode for rollFork_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { forkId, txHash } = self;
        persist_caller(ccx);
        let result = fork_op(ccx, |db, evm| {
            db.roll_fork_to_transaction(Some(*forkId), *txHash, evm, ExecutionPhase::InFlight)
        })?;
        record_fork_roll(ccx, Some(*forkId));
        Ok(result)
    }
}

impl Cheatcode for selectForkCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { forkId } = self;
        persist_caller(ccx);
        check_broadcast(ccx.state)?;
        let source_fork_id = ccx.active_fork_id();
        let initial = ccx.state.created_account_bindings(None);
        let propagated = persistent_created_accounts(ccx);
        let result =
            fork_op(ccx, |db, evm| db.select_fork(*forkId, evm, ExecutionPhase::InFlight))?;
        record_fork_switch(ccx, source_fork_id, initial, propagated);
        Ok(result)
    }
}

impl Cheatcode for transact_0Call {
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let Self { txHash } = *self;
        transact(ccx, executor, txHash, None)
    }
}

impl Cheatcode for transact_1Call {
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let Self { forkId, txHash } = *self;
        transact(ccx, executor, txHash, Some(forkId))
    }
}

impl Cheatcode for allowCheatcodesCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        ccx.db_mut().allow_cheatcode_access(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        ccx.db_mut().add_persistent_account(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account0, account1 } = self;
        ccx.db_mut().add_persistent_account(*account0);
        ccx.db_mut().add_persistent_account(*account1);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account0, account1, account2 } = self;
        ccx.db_mut().add_persistent_account(*account0);
        ccx.db_mut().add_persistent_account(*account1);
        ccx.db_mut().add_persistent_account(*account2);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { accounts } = self;
        for account in accounts {
            ccx.db_mut().add_persistent_account(*account);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        ccx.db_mut().remove_persistent_account(account);
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { accounts } = self;
        for account in accounts {
            ccx.db_mut().remove_persistent_account(account);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for isPersistentCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        Ok(ccx.is_persistent(account).abi_encode())
    }
}

impl Cheatcode for rpc_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { method, params } = self;
        let url = ccx.active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let result = rpc_call(&url, method, params)?;
        refresh_active_fork_state(ccx, method, params)?;
        Ok(result)
    }
}

impl Cheatcode for rpc_1Call {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self { urlOrAlias, method, params } = self;
        let url = state.config.rpc_endpoint(urlOrAlias)?.url()?;
        rpc_call(&url, method, params)
    }
}

impl Cheatcode for rpcJson_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { method, params } = self;
        let url = ccx.active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let result = rpc_json_call(&url, method, params)?;
        refresh_active_fork_state(ccx, method, params)?;
        Ok(result)
    }
}

impl Cheatcode for rpcJson_1Call {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self { urlOrAlias, method, params } = self;
        let url = state.config.rpc_endpoint(urlOrAlias)?.url()?;
        rpc_json_call(&url, method, params)
    }
}

impl Cheatcode for eth_getLogsCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { fromBlock, toBlock, target, topics } = self;
        let (Ok(from_block), Ok(to_block)) = (u64::try_from(fromBlock), u64::try_from(toBlock))
        else {
            bail!("blocks in block range must be less than 2^64")
        };

        if topics.len() > 4 {
            bail!("topics array must contain at most 4 elements")
        }

        let provider = active_fork_provider(ccx)?;
        let mut filter = Filter::new().address(*target).from_block(from_block).to_block(to_block);
        for (i, &topic) in topics.iter().enumerate() {
            filter.topics[i] = topic.into();
        }

        let logs = foundry_common::block_on(provider.get_logs(&filter))
            .map_err(|e| fmt_err!("failed to get logs: {e}"))?;

        let eth_logs = logs
            .into_iter()
            .map(|log| EthGetLogs {
                emitter: log.address(),
                topics: log.topics().to_vec(),
                data: log.inner.data.data,
                blockHash: log.block_hash.unwrap_or_default(),
                blockNumber: log.block_number.unwrap_or_default(),
                transactionHash: log.transaction_hash.unwrap_or_default(),
                transactionIndex: log.transaction_index.unwrap_or_default(),
                logIndex: U256::from(log.log_index.unwrap_or_default()),
                removed: log.removed,
            })
            .collect::<Vec<_>>();

        Ok(eth_logs.abi_encode())
    }
}

impl Cheatcode for eth_getProofCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slots, blockNumber } = self;
        let block_number = u64::try_from(blockNumber)
            .map_err(|_| fmt_err!("block number must be less than 2^64"))?;

        let provider = active_fork_provider(ccx)?;
        let proof = foundry_common::block_on(async move {
            provider.get_proof(*target, slots.clone()).number(block_number).await
        })
        .map_err(|e| fmt_err!("failed to get proof: {e}"))?;

        let storage_proof = proof
            .storage_proof
            .into_iter()
            .map(|p| EthStorageProof { key: p.key.as_b256(), value: p.value, proof: p.proof })
            .collect();

        Ok(EthGetProof {
            account: proof.address,
            balance: proof.balance,
            codeHash: proof.code_hash,
            nonce: proof.nonce,
            storageHash: proof.storage_hash,
            accountProof: proof.account_proof,
            storageProof: storage_proof,
        }
        .abi_encode())
    }
}

impl Cheatcode for getRawBlockHeaderCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { blockNumber } = self;
        let provider = active_fork_provider(ccx)?;
        let block_number = u64::try_from(blockNumber)
            .map_err(|_| fmt_err!("block number must be less than 2^64"))?;
        let block =
            foundry_common::block_on(async move { provider.get_block(block_number.into()).await })
                .map_err(|e| fmt_err!("failed to get block: {e}"))?
                .ok_or_else(|| fmt_err!("block {block_number} not found"))?;

        let header: alloy_consensus::Header = block
            .into_inner()
            .header
            .inner
            .try_into_header()
            .map_err(|e| fmt_err!("failed to convert to header: {e}"))?;
        Ok(alloy_rlp::encode(&header).abi_encode())
    }
}

/// Creates and then also selects the new fork
fn create_select_fork<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result {
    check_broadcast(ccx.state)?;

    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let source_fork_id = ccx.active_fork_id();
    let initial = ccx.state.created_account_bindings(None);
    let propagated = persistent_created_accounts(ccx);
    let result =
        fork_op(ccx, |db, evm| db.create_select_fork(fork, evm, ExecutionPhase::InFlight))?;
    record_fork_switch(ccx, source_fork_id, initial, propagated);
    Ok(result)
}

/// Creates a new fork
fn create_fork<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let id = ccx.db_mut().create_fork(fork)?;
    Ok(id.abi_encode())
}

/// Creates and then also selects the new fork at the given transaction
fn create_select_fork_at_transaction<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    check_broadcast(ccx.state)?;

    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let source_fork_id = ccx.active_fork_id();
    let initial = ccx.state.created_account_bindings(None);
    let propagated = persistent_created_accounts(ccx);
    let result = fork_op(ccx, |db, evm| {
        db.create_select_fork_at_transaction(fork, evm, ExecutionPhase::InFlight, *transaction)
    })?;
    record_fork_switch(ccx, source_fork_id, initial, propagated);
    Ok(result)
}

/// Creates a new fork at the given transaction
fn create_fork_at_transaction<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let id = ccx.db_mut().create_fork_at_transaction(fork, *transaction)?;
    Ok(id.abi_encode())
}

/// Creates the request object for a new fork request
fn create_fork_request<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result<CreateFork> {
    persist_caller(ccx);

    let rpc_endpoint = ccx.state.core.config.rpc_endpoint(url_or_alias)?;
    let url = rpc_endpoint.url()?;
    let mut evm_opts = ccx.state.core.config.evm_opts.clone();
    evm_opts.fork_block_number = block;
    evm_opts.fork_block_number_is_inferred = false;
    evm_opts.fork_retries = rpc_endpoint.config.retries;
    evm_opts.fork_retry_backoff = rpc_endpoint.config.retry_backoff;
    if let Some(Ok(auth)) = rpc_endpoint.auth {
        let fork_headers = Some(vec![format!("Authorization: {auth}")]);
        // The cached identity was discovered with other credentials, which may reach another
        // backend behind the same URL.
        if evm_opts.fork_headers != fork_headers {
            evm_opts.fork_endpoint = None;
        }
        evm_opts.fork_headers = fork_headers;
    }
    let fork = CreateFork {
        enable_caching: !ccx.state.core.config.no_storage_caching
            && ccx.state.core.config.rpc_storage_caching.enable_for_endpoint(&url),
        url,
        evm_opts,
    };
    Ok(fork)
}

/// Runs a fork operation that switches or replaces the running context, such as `rollFork`,
/// `selectFork` or `createSelectFork`.
fn fork_op<FEN: FoundryEvmNetwork, T: SolValue>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    f: impl FnOnce(&mut Backend<FEN>, &mut FoundryEvmFor<'_, FEN>) -> eyre::Result<T>,
) -> Result {
    ensure!(!ccx.state.in_replay, "cannot select or roll forks in a replayed transaction");
    let (evm, db) = ccx.evm_db_mut();
    let result = f(db, evm)?;
    ccx.state.state_epoch += 1;
    ccx.state.fork_epoch += 1;
    Ok(result.abi_encode())
}

fn persistent_created_accounts<FEN: FoundryEvmNetwork>(
    ccx: &CheatsCtxt<'_, '_, FEN>,
) -> Vec<(Address, usize)> {
    let fork_id = ccx.active_fork_id();
    ccx.state
        .created_account_bindings(fork_id)
        .into_iter()
        .filter(|(address, _)| ccx.is_persistent(address))
        .collect()
}

fn record_fork_switch<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    source_fork_id: Option<LocalForkId>,
    initial: AddressHashMap<usize>,
    propagated: Vec<(Address, usize)>,
) {
    let target_fork_id = ccx.active_fork_id();
    ccx.state.fork_block_number_override = ccx.active_fork_block_number();
    if source_fork_id != target_fork_id {
        ccx.state.commit_created_account_changes(source_fork_id);
        // The switch accepted the running state, so reverts no longer undo its persistent writes.
        ccx.state.persistent_writes.clear();
    }
    ccx.state.record_initial_created_accounts(target_fork_id, initial);
    ccx.state.record_propagated_accounts(target_fork_id, propagated);
    sync_chain_id_override(ccx, target_fork_id);
}

fn record_fork_roll<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    target_fork_id: Option<LocalForkId>,
) {
    let active_fork_id = ccx.active_fork_id();
    let rolled_fork_id = target_fork_id.or(active_fork_id);
    ccx.state.env_overrides.clear_implicit_basefee(rolled_fork_id);
    if target_fork_id.is_none() || target_fork_id == active_fork_id {
        ccx.state.fork_block_number_override = ccx.active_fork_block_number();
        ccx.state.commit_created_account_changes(active_fork_id);
        // The roll accepted the running state, so reverts no longer undo its persistent writes.
        ccx.state.persistent_writes.clear();
        sync_chain_id_override(ccx, active_fork_id);
    }
}

/// Makes `CHAINID` on the fork `fork_id` just switched to or rolled read the fork's chain ID at
/// once, replacing an earlier `vm.chainId`.
///
/// The switch or roll defers the chain ID to the next transaction, and `CHAINID` reads the
/// transaction's until then. `ccx.evm` can be a frame EVM that already runs the pending chain ID,
/// so the pending chain ID is recorded whenever there is one.
fn sync_chain_id_override<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    fork_id: Option<LocalForkId>,
) {
    let pending = ccx.db().lifecycle().pending_env().map(|env| env.chain_id());
    if pending.is_none()
        && !ccx.state.env_overrides.get(fork_id).is_some_and(|o| o.chain_id.is_some())
    {
        return;
    }
    let chain_id = pending.unwrap_or_else(|| ccx.evm.version().chain_id);
    ccx.state.env_overrides.update(fork_id, |o| o.chain_id = Some(chain_id));
}

fn check_broadcast<FEN: FoundryEvmNetwork>(state: &Cheatcodes<FEN>) -> Result<()> {
    if state.broadcast.is_none() {
        Ok(())
    } else {
        Err(fmt_err!("cannot select forks during a broadcast"))
    }
}

fn transact<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    executor: &mut dyn CheatcodesExecutor<FEN>,
    transaction: B256,
    fork_id: Option<U256>,
) -> Result {
    let _ = executor.transact(
        ccx.state,
        ccx.backend,
        ccx.evm,
        ccx.depth as u16,
        fork_id,
        transaction,
    )?;
    ccx.state.state_epoch += 1;
    Ok(Default::default())
}

// Helper to add the caller of fork cheat code as persistent account (in order to make sure that the
// state of caller contract is not lost when fork changes).
// Applies to create, select and roll forks actions.
// https://github.com/foundry-rs/foundry/issues/8004
fn persist_caller<FEN: FoundryEvmNetwork>(ccx: &mut CheatsCtxt<'_, '_, FEN>) {
    let caller = ccx.caller;
    ccx.db_mut().add_persistent_account(caller);
}

/// Returns a provider for the active fork, configured with the RPC options the fork was created
/// with (e.g. auth headers, retries and timeouts).
fn active_fork_provider<FEN: FoundryEvmNetwork>(
    ccx: &CheatsCtxt<'_, '_, FEN>,
) -> Result<RootProvider<AnyNetwork>> {
    let fork = ccx.active_fork_options().ok_or_else(|| fmt_err!("no active fork URL found"))?;
    Ok(fork.evm_opts.fork_provider_with_url(&fork.url)?)
}

/// Performs an Ethereum JSON-RPC request to the given endpoint.
fn rpc_call(url: &str, method: &str, params: &str) -> Result {
    let result = rpc_result(url, method, params)?;
    let result_as_tokens = convert_to_bytes(
        &json_value_to_token(&result, None)
            .map_err(|err| fmt_err!("failed to parse result: {err}"))?,
    );

    let payload = match &result_as_tokens {
        DynSolValue::Bytes(b) => b.clone(),
        _ => result_as_tokens.abi_encode(),
    };
    Ok(DynSolValue::Bytes(payload).abi_encode())
}

/// Performs an Ethereum JSON-RPC request to the given endpoint and returns the JSON result.
fn rpc_json_call(url: &str, method: &str, params: &str) -> Result {
    let result = rpc_result(url, method, params)?;
    Ok(serde_json::to_string(&result)?.abi_encode())
}

/// Refreshes cached fork and journal state after a `vm.rpc` call on the active fork that mutates
/// the node directly. Only well-known Anvil/Hardhat account/storage setters are handled;
/// chain-advancing methods (e.g. `eth_sendTransaction`) need re-forking, not synchronization.
fn refresh_active_fork_state<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    method: &str,
    params: &str,
) -> Result<()> {
    let account_field = match method {
        "anvil_setBalance"
        | "hardhat_setBalance"
        | "tenderly_setBalance"
        | "anvil_addBalance"
        | "hardhat_addBalance"
        | "tenderly_addBalance" => Some(ForkAccountField::Balance),
        "anvil_setNonce" | "hardhat_setNonce" | "evm_setAccountNonce" => {
            Some(ForkAccountField::Nonce)
        }
        "anvil_setCode" | "hardhat_setCode" => Some(ForkAccountField::Code),
        "anvil_setStorageAt" | "hardhat_setStorageAt" => None,
        _ => return Ok(()),
    };

    let Ok(params) = serde_json::from_str::<serde_json::Value>(params) else { return Ok(()) };
    let Some(address) =
        params.get(0).and_then(|v| v.as_str()).and_then(|s| s.parse::<Address>().ok())
    else {
        return Ok(());
    };

    let (evm, db) = ccx.evm_db_mut();
    if let Some(field) = account_field {
        db.refresh_fork_account(address, field, evm)?;
    } else {
        let Some(slot) =
            params.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse::<U256>().ok())
        else {
            return Ok(());
        };
        if let Some(write) = db.refresh_fork_storage(address, slot, evm)? {
            ccx.state.record_persistent(ccx.caller_depth(), ccx.evm.state(), [write]);
        }
    }
    Ok(())
}

fn rpc_result(url: &str, method: &str, params: &str) -> Result<serde_json::Value> {
    let provider = ProviderBuilder::<AnyNetwork>::new(url).build()?;
    let params_json: serde_json::Value = serde_json::from_str(params)?;
    foundry_common::block_on(provider.raw_request(method.to_string().into(), params_json))
        .map_err(|err| fmt_err!("{method:?}: {err}"))
}

/// Convert fixed bytes and address values to bytes in order to prevent encoding issues.
fn convert_to_bytes(token: &DynSolValue) -> DynSolValue {
    match token {
        // Convert fixed bytes to prevent encoding issues.
        // See: <https://github.com/foundry-rs/foundry/issues/8287>
        DynSolValue::FixedBytes(bytes, size) => {
            DynSolValue::Bytes(bytes.as_slice()[..*size].to_vec())
        }
        DynSolValue::Address(addr) => DynSolValue::Bytes(addr.to_vec()),
        val => val.clone(),
    }
}
