use crate::{
    Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, DatabaseExt, Result, Vm::*,
    json::json_value_to_token,
};
use alloy_dyn_abi::DynSolValue;
use alloy_evm::EvmEnv;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types::Filter;
use alloy_sol_types::SolValue;
use foundry_common::provider::ProviderBuilder;
use foundry_evm_core::{
    FoundryContextExt, backend::JournaledState, evm::FoundryEvmNetwork, fork::CreateFork,
};
use revm::context::ContextTr;

impl Cheatcode for activeForkCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.ecx
            .db()
            .active_fork_id()
            .map(|id| id.abi_encode())
            .ok_or_else(|| fmt_err!("no active fork"))
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
        fork_env_op(ccx.ecx, |db, evm_env, _, inner| {
            db.roll_fork(None, (*blockNumber).to(), evm_env, inner)
        })
    }
}

impl Cheatcode for rollFork_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { txHash } = self;
        persist_caller(ccx);
        fork_env_op(ccx.ecx, |db, evm_env, _, inner| {
            db.roll_fork_to_transaction(None, *txHash, evm_env, inner)
        })
    }
}

impl Cheatcode for rollFork_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { forkId, blockNumber } = self;
        persist_caller(ccx);
        fork_env_op(ccx.ecx, |db, evm_env, _, inner| {
            db.roll_fork(Some(*forkId), (*blockNumber).to(), evm_env, inner)
        })
    }
}

impl Cheatcode for rollFork_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { forkId, txHash } = self;
        persist_caller(ccx);
        fork_env_op(ccx.ecx, |db, evm_env, _, inner| {
            db.roll_fork_to_transaction(Some(*forkId), *txHash, evm_env, inner)
        })
    }
}

impl Cheatcode for selectForkCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { forkId } = self;
        persist_caller(ccx);
        check_broadcast(ccx.state)?;
        fork_env_op(ccx.ecx, |db, evm_env, tx_env, inner| {
            db.select_fork(*forkId, evm_env, tx_env, inner)
        })
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
        ccx.ecx.db_mut().allow_cheatcode_access(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        ccx.ecx.db_mut().add_persistent_account(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account0, account1 } = self;
        ccx.ecx.db_mut().add_persistent_account(*account0);
        ccx.ecx.db_mut().add_persistent_account(*account1);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account0, account1, account2 } = self;
        ccx.ecx.db_mut().add_persistent_account(*account0);
        ccx.ecx.db_mut().add_persistent_account(*account1);
        ccx.ecx.db_mut().add_persistent_account(*account2);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { accounts } = self;
        for account in accounts {
            ccx.ecx.db_mut().add_persistent_account(*account);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        ccx.ecx.db_mut().remove_persistent_account(account);
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { accounts } = self;
        for account in accounts {
            ccx.ecx.db_mut().remove_persistent_account(account);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for isPersistentCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        Ok(ccx.ecx.db().is_persistent(account).abi_encode())
    }
}

impl Cheatcode for rpc_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { method, params } = self;
        let url =
            ccx.ecx.db().active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let result = rpc_call(&url, method, params)?;
        invalidate_active_fork_cache(ccx, method, params);
        Ok(result)
    }
}

impl Cheatcode for rpc_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { urlOrAlias, method, params } = self;
        let url = state.config.rpc_endpoint(urlOrAlias)?.url()?;
        rpc_call(&url, method, params)
    }
}

impl Cheatcode for rpcJson_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { method, params } = self;
        let url =
            ccx.ecx.db().active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let result = rpc_json_call(&url, method, params)?;
        invalidate_active_fork_cache(ccx, method, params);
        Ok(result)
    }
}

impl Cheatcode for rpcJson_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
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

        let url =
            ccx.ecx.db().active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let provider = ProviderBuilder::<AnyNetwork>::new(&url).build()?;
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

impl Cheatcode for getRawBlockHeaderCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { blockNumber } = self;
        let url = ccx.ecx.db().active_fork_url().ok_or_else(|| fmt_err!("no active fork"))?;
        let provider = ProviderBuilder::<AnyNetwork>::new(&url).build()?;
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
    fork_env_op(ccx.ecx, |db, evm_env, tx_env, inner| {
        db.create_select_fork(fork, evm_env, tx_env, inner)
    })
}

/// Creates a new fork
fn create_fork<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let id = ccx.ecx.db_mut().create_fork(fork)?;
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
    fork_env_op(ccx.ecx, |db, evm_env, tx_env, inner| {
        db.create_select_fork_at_transaction(fork, evm_env, tx_env, inner, *transaction)
    })
}

/// Creates a new fork at the given transaction
fn create_fork_at_transaction<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let id = ccx.ecx.db_mut().create_fork_at_transaction(fork, *transaction)?;
    Ok(id.abi_encode())
}

/// Creates the request object for a new fork request
fn create_fork_request<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result<CreateFork> {
    persist_caller(ccx);

    let rpc_endpoint = ccx.state.config.rpc_endpoint(url_or_alias)?;
    let url = rpc_endpoint.url()?;
    let mut evm_opts = ccx.state.config.evm_opts.clone();
    evm_opts.fork_block_number = block;
    evm_opts.fork_retries = rpc_endpoint.config.retries;
    evm_opts.fork_retry_backoff = rpc_endpoint.config.retry_backoff;
    if let Some(Ok(auth)) = rpc_endpoint.auth {
        evm_opts.fork_headers = Some(vec![format!("Authorization: {auth}")]);
    }
    let fork = CreateFork {
        enable_caching: !ccx.state.config.no_storage_caching
            && ccx.state.config.rpc_storage_caching.enable_for_endpoint(&url),
        url,
        evm_opts,
    };
    Ok(fork)
}

/// Clones the EVM and tx environments, runs a fork operation that may modify them, then writes
/// them back. This is the common pattern for all fork-switching cheatcodes (rollFork, selectFork,
/// createSelectFork).
fn fork_env_op<CTX: FoundryContextExt, T: SolValue>(
    ecx: &mut CTX,
    f: impl FnOnce(
        &mut CTX::Db,
        &mut EvmEnv<CTX::Spec, CTX::Block>,
        &mut CTX::Tx,
        &mut JournaledState,
    ) -> eyre::Result<T>,
) -> Result {
    let mut evm_env = ecx.evm_clone();
    let mut tx_env = ecx.tx_clone();
    let (db, inner) = ecx.db_journal_inner_mut();
    let result = f(db, &mut evm_env, &mut tx_env, inner)?;
    ecx.set_evm(evm_env);
    ecx.set_tx(tx_env);
    Ok(result.abi_encode())
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
    executor.transact_on_db(ccx.state, ccx.ecx, fork_id, transaction)?;
    Ok(Default::default())
}

// Helper to add the caller of fork cheat code as persistent account (in order to make sure that the
// state of caller contract is not lost when fork changes).
// Applies to create, select and roll forks actions.
// https://github.com/foundry-rs/foundry/issues/8004
fn persist_caller<FEN: FoundryEvmNetwork>(ccx: &mut CheatsCtxt<'_, '_, FEN>) {
    ccx.ecx.db_mut().add_persistent_account(ccx.caller);
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

/// Invalidates the fork DB cache after a `vm.rpc` call on the active fork that mutates the node
/// directly (bypassing the cache), so later DB reads (e.g. the broadcast simulation runner)
/// re-fetch the new value. Only well-known Anvil/Hardhat account/storage setters are handled;
/// chain-advancing methods (e.g. `eth_sendTransaction`) need re-forking, not eviction.
fn invalidate_active_fork_cache<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    method: &str,
    params: &str,
) {
    const ACCOUNT_SETTERS: &[&str] = &[
        "anvil_setBalance",
        "hardhat_setBalance",
        "tenderly_setBalance",
        "anvil_addBalance",
        "hardhat_addBalance",
        "tenderly_addBalance",
        "anvil_setNonce",
        "hardhat_setNonce",
        "evm_setAccountNonce",
        "anvil_setCode",
        "hardhat_setCode",
    ];
    let is_storage_setter = matches!(method, "anvil_setStorageAt" | "hardhat_setStorageAt");
    if !is_storage_setter && !ACCOUNT_SETTERS.contains(&method) {
        return;
    }

    let Ok(params) = serde_json::from_str::<serde_json::Value>(params) else { return };
    let Some(address) =
        params.get(0).and_then(|v| v.as_str()).and_then(|s| s.parse::<Address>().ok())
    else {
        return;
    };

    if is_storage_setter {
        let Some(slot) =
            params.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse::<U256>().ok())
        else {
            return;
        };
        ccx.ecx.db_mut().invalidate_fork_cache_storage(address, slot);
    } else {
        ccx.ecx.db_mut().invalidate_fork_cache_account(address);
    }
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
