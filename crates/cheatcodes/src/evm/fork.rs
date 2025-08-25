use crate::{
    Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, DatabaseExt, Result, Vm::*,
    json::json_value_to_token,
};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{B256, U256};
use alloy_provider::Provider;
use alloy_rpc_types::Filter;
use alloy_sol_types::SolValue;
use foundry_common::provider::ProviderBuilder;
use foundry_evm_core::{AsEnvMut, ContextExt, fork::CreateFork};

impl Cheatcode for activeForkCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        ccx.ecx
            .journaled_state
            .database
            .active_fork_id()
            .map(|id| id.abi_encode())
            .ok_or_else(|| fmt_err!("no active fork"))
    }
}

impl Cheatcode for createFork_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { urlOrAlias } = self;
        create_fork(ccx, urlOrAlias, None)
    }
}

impl Cheatcode for createFork_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { urlOrAlias, blockNumber } = self;
        create_fork(ccx, urlOrAlias, Some(blockNumber.saturating_to()))
    }
}

impl Cheatcode for createFork_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { urlOrAlias, txHash } = self;
        create_fork_at_transaction(ccx, urlOrAlias, txHash)
    }
}

impl Cheatcode for createSelectFork_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { urlOrAlias } = self;
        create_select_fork(ccx, urlOrAlias, None)
    }
}

impl Cheatcode for createSelectFork_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { urlOrAlias, blockNumber } = self;
        create_select_fork(ccx, urlOrAlias, Some(blockNumber.saturating_to()))
    }
}

impl Cheatcode for createSelectFork_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { urlOrAlias, txHash } = self;
        create_select_fork_at_transaction(ccx, urlOrAlias, txHash)
    }
}

impl Cheatcode for rollFork_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { blockNumber } = self;
        persist_caller(ccx);
        let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
        db.roll_fork(None, (*blockNumber).to(), &mut env, journal)?;
        Ok(Default::default())
    }
}

impl Cheatcode for rollFork_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { txHash } = self;
        persist_caller(ccx);
        let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
        db.roll_fork_to_transaction(None, *txHash, &mut env, journal)?;
        Ok(Default::default())
    }
}

impl Cheatcode for rollFork_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { forkId, blockNumber } = self;
        persist_caller(ccx);
        let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
        db.roll_fork(Some(*forkId), (*blockNumber).to(), &mut env, journal)?;
        Ok(Default::default())
    }
}

impl Cheatcode for rollFork_3Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { forkId, txHash } = self;
        persist_caller(ccx);
        let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
        db.roll_fork_to_transaction(Some(*forkId), *txHash, &mut env, journal)?;
        Ok(Default::default())
    }
}

impl Cheatcode for selectForkCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { forkId } = self;
        persist_caller(ccx);
        check_broadcast(ccx.state)?;
        let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
        db.select_fork(*forkId, &mut env, journal)?;
        Ok(Default::default())
    }
}

impl Cheatcode for transact_0Call {
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
        let Self { txHash } = *self;
        transact(ccx, executor, txHash, None)
    }
}

impl Cheatcode for transact_1Call {
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
        let Self { forkId, txHash } = *self;
        transact(ccx, executor, txHash, Some(forkId))
    }
}

impl Cheatcode for allowCheatcodesCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account } = self;
        ccx.ecx.journaled_state.database.allow_cheatcode_access(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account } = self;
        ccx.ecx.journaled_state.database.add_persistent_account(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account0, account1 } = self;
        ccx.ecx.journaled_state.database.add_persistent_account(*account0);
        ccx.ecx.journaled_state.database.add_persistent_account(*account1);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account0, account1, account2 } = self;
        ccx.ecx.journaled_state.database.add_persistent_account(*account0);
        ccx.ecx.journaled_state.database.add_persistent_account(*account1);
        ccx.ecx.journaled_state.database.add_persistent_account(*account2);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_3Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { accounts } = self;
        for account in accounts {
            ccx.ecx.journaled_state.database.add_persistent_account(*account);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account } = self;
        ccx.ecx.journaled_state.database.remove_persistent_account(account);
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { accounts } = self;
        for account in accounts {
            ccx.ecx.journaled_state.database.remove_persistent_account(account);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for isPersistentCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account } = self;
        Ok(ccx.ecx.journaled_state.database.is_persistent(account).abi_encode())
    }
}

impl Cheatcode for rpc_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { method, params } = self;
        let url = ccx
            .ecx
            .journaled_state
            .database
            .active_fork_url()
            .ok_or_else(|| fmt_err!("no active fork URL found"))?;
        rpc_call(&url, method, params)
    }
}

impl Cheatcode for rpc_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { urlOrAlias, method, params } = self;
        let url = state.config.rpc_endpoint(urlOrAlias)?.url()?;
        rpc_call(&url, method, params)
    }
}

impl Cheatcode for eth_getLogsCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { fromBlock, toBlock, target, topics } = self;
        let (Ok(from_block), Ok(to_block)) = (u64::try_from(fromBlock), u64::try_from(toBlock))
        else {
            bail!("blocks in block range must be less than 2^64 - 1")
        };

        if topics.len() > 4 {
            bail!("topics array must contain at most 4 elements")
        }

        let url = ccx
            .ecx
            .journaled_state
            .database
            .active_fork_url()
            .ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let provider = ProviderBuilder::new(&url).build()?;
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
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { blockNumber } = self;
        let url = ccx
            .ecx
            .journaled_state
            .database
            .active_fork_url()
            .ok_or_else(|| fmt_err!("no active fork"))?;
        let provider = ProviderBuilder::new(&url).build()?;
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
fn create_select_fork(ccx: &mut CheatsCtxt, url_or_alias: &str, block: Option<u64>) -> Result {
    check_broadcast(ccx.state)?;

    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
    let id = db.create_select_fork(fork, &mut env, journal)?;
    Ok(id.abi_encode())
}

/// Creates a new fork
fn create_fork(ccx: &mut CheatsCtxt, url_or_alias: &str, block: Option<u64>) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let id = ccx.ecx.journaled_state.database.create_fork(fork)?;
    Ok(id.abi_encode())
}

/// Creates and then also selects the new fork at the given transaction
fn create_select_fork_at_transaction(
    ccx: &mut CheatsCtxt,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    check_broadcast(ccx.state)?;

    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
    let id = db.create_select_fork_at_transaction(fork, &mut env, journal, *transaction)?;
    Ok(id.abi_encode())
}

/// Creates a new fork at the given transaction
fn create_fork_at_transaction(
    ccx: &mut CheatsCtxt,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let id = ccx.ecx.journaled_state.database.create_fork_at_transaction(fork, *transaction)?;
    Ok(id.abi_encode())
}

/// Creates the request object for a new fork request
fn create_fork_request(
    ccx: &mut CheatsCtxt,
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
        env: ccx.ecx.as_env_mut().to_owned(),
        evm_opts,
    };
    Ok(fork)
}

fn check_broadcast(state: &Cheatcodes) -> Result<()> {
    if state.broadcast.is_none() {
        Ok(())
    } else {
        Err(fmt_err!("cannot select forks during a broadcast"))
    }
}

fn transact(
    ccx: &mut CheatsCtxt,
    executor: &mut dyn CheatcodesExecutor,
    transaction: B256,
    fork_id: Option<U256>,
) -> Result {
    let (db, journal, env) = ccx.ecx.as_db_env_and_journal();
    db.transact(
        fork_id,
        transaction,
        env.to_owned(),
        journal,
        &mut *executor.get_inspector(ccx.state),
    )?;
    Ok(Default::default())
}

// Helper to add the caller of fork cheat code as persistent account (in order to make sure that the
// state of caller contract is not lost when fork changes).
// Applies to create, select and roll forks actions.
// https://github.com/foundry-rs/foundry/issues/8004
fn persist_caller(ccx: &mut CheatsCtxt) {
    ccx.ecx.journaled_state.database.add_persistent_account(ccx.caller);
}

/// Performs an Ethereum JSON-RPC request to the given endpoint.
fn rpc_call(url: &str, method: &str, params: &str) -> Result {
    let provider = ProviderBuilder::new(url).build()?;
    let params_json: serde_json::Value = serde_json::from_str(params)?;
    let result =
        foundry_common::block_on(provider.raw_request(method.to_string().into(), params_json))
            .map_err(|err| fmt_err!("{method:?}: {err}"))?;
    let result_as_tokens = convert_to_bytes(
        &json_value_to_token(&result).map_err(|err| fmt_err!("failed to parse result: {err}"))?,
    );

    Ok(result_as_tokens.abi_encode())
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
        //  Convert tuple values to prevent encoding issues.
        // See: <https://github.com/foundry-rs/foundry/issues/7858>
        DynSolValue::Tuple(vals) => DynSolValue::Tuple(vals.iter().map(convert_to_bytes).collect()),
        val => val.clone(),
    }
}
